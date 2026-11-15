//! Signatures (§3.5): ed25519 signatures of the ODI digest, in a block of
//! their own outside the digest, so a file is signed after it's built and
//! signing it changes no byte the digest covers.

use ed25519_dalek::Signer;
use serde::{Deserialize, Serialize};

use crate::{bail, Result};

/// What's signed: this, then the 32 bytes of the ODI digest.
pub const SIGNED_PREFIX: &[u8] = b"odi-signature-v0\n";

/// The largest signatures block a reader takes.
pub(crate) const MAX_SIGNATURES: u64 = 64 << 10;

/// One signature (§3.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    /// `ed25519`.
    pub alg: String,
    /// The public key: `ed25519:` and 64 hex digits.
    pub key: String,
    /// The signature, 128 hex digits.
    pub sig: String,
}

/// The signatures block: `{"signatures": [...]}`, compressed with zstd.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signatures {
    #[serde(default)]
    pub signatures: Vec<Signature>,
}

fn message(digest: &[u8; 32]) -> Vec<u8> {
    [SIGNED_PREFIX, &digest[..]].concat()
}

/// `ed25519:` and the key's hex: how a public key is written.
pub fn key_string(public: &[u8; 32]) -> String {
    format!("ed25519:{}", hex::encode(public))
}

/// A public key written as [`key_string`] writes it.
pub fn parse_key(s: &str) -> Result<[u8; 32]> {
    let Some(h) = s.strip_prefix("ed25519:") else { bail!("key {s:?}: ed25519: and 64 hex digits") };
    let mut k = [0u8; 32];
    if hex::decode_to_slice(h, &mut k).is_err() {
        bail!("key {s:?}: ed25519: and 64 hex digits");
    }
    Ok(k)
}

/// Signs a digest with a secret key (its 32 bytes).
pub fn sign(secret: &[u8; 32], digest: &[u8; 32]) -> Signature {
    let k = ed25519_dalek::SigningKey::from_bytes(secret);
    let sig = k.sign(&message(digest));
    Signature {
        alg: "ed25519".into(),
        key: key_string(&k.verifying_key().to_bytes()),
        sig: hex::encode(sig.to_bytes()),
    }
}

/// The public key of a secret key.
pub fn public_key(secret: &[u8; 32]) -> [u8; 32] {
    ed25519_dalek::SigningKey::from_bytes(secret).verifying_key().to_bytes()
}

impl Signature {
    /// Checks the signature is its key's, of this digest.
    pub fn check(&self, digest: &[u8; 32]) -> Result<()> {
        if self.alg != "ed25519" {
            bail!("a signature in {:?}: this reader knows ed25519", self.alg);
        }
        let key = parse_key(&self.key)?;
        let mut sig = [0u8; 64];
        if hex::decode_to_slice(&self.sig, &mut sig).is_err() {
            bail!("a signature that isn't 128 hex digits");
        }
        let vk = ed25519_dalek::VerifyingKey::from_bytes(&key).map_err(|_| crate::Error("a bad key".into()))?;
        if vk.verify_strict(&message(digest), &ed25519_dalek::Signature::from_bytes(&sig)).is_err() {
            bail!("the signature by {} doesn't match", self.key);
        }
        Ok(())
    }
}

#[cfg(feature = "zstd")]
/// Writes rd's file again to out with these signatures (in place of any it
/// had): every byte up to them as it was, then the signatures, then the
/// footer pointing at them. The digest stays the same.
pub fn write_signed(rd: &crate::Reader, sigs: &Signatures, out: &mut dyn std::io::Write) -> Result<()> {
    use crate::FOOTER_SIZE;
    let size = rd.size();
    let mut f = [0u8; FOOTER_SIZE as usize];
    rd.read_at(&mut f, size - FOOTER_SIZE)?;
    let field = |i: usize| u64::from_le_bytes(f[i..i + 8].try_into().expect("8 bytes"));
    // What's kept: everything up to the end of the last thing a reader needs.
    let mut keep = (field(0) + field(8)).max(field(16) + field(24));
    for e in &rd.entries {
        for c in &e.chunks {
            keep = keep.max(c.offset + c.length);
        }
    }
    let old = field(32);
    if old != 0 && old < keep {
        bail!("can't sign this file: its signatures aren't after everything else");
    }
    let mut buf = vec![0u8; 1 << 20];
    let mut at = 0u64;
    while at < keep {
        let n = (keep - at).min(buf.len() as u64) as usize;
        rd.read_at(&mut buf[..n], at)?;
        out.write_all(&buf[..n])?;
        at += n as u64;
    }
    let block = zstd::bulk::compress(&serde_json::to_vec(sigs)?, 7)?;
    if block.len() as u64 > MAX_SIGNATURES {
        bail!("too many signatures");
    }
    out.write_all(&block)?;
    f[32..40].copy_from_slice(&keep.to_le_bytes());
    f[40..48].copy_from_slice(&(block.len() as u64).to_le_bytes());
    out.write_all(&f)?;
    Ok(())
}
