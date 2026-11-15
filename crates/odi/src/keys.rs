//! Signing (SPEC §3.5): keys, `odi sign`, and which signers a runner trusts.
//!
//! - Secret keys are in ~/.odi/keys/NAME.key (hex, readable by their owner
//!   only); `odi key new` makes one.
//! - Trusted public keys: ~/.odi/trust (one per line, `#` comments), $ODI_TRUST
//!   (comma-separated) and `odi run --trust KEY`. With any trusted, `odi run`
//!   runs only a file one of them signed. A set's signature covers its parts:
//!   it pins each by digest.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use odi_format::{digest_string, key_string, parse_key, public_key, sign, write_signed, Reader, Signatures};

use crate::flags::{self, flag, Kind::*};

fn keys_dir() -> PathBuf {
    crate::hub::odi_home().join("keys")
}

fn secret(name: &str) -> Result<[u8; 32]> {
    if !odi_format::valid_name(name) {
        bail!("key {name:?}: letters, digits, . - _");
    }
    let p = keys_dir().join(format!("{name}.key"));
    let h = fs::read_to_string(&p).with_context(|| format!("key {name} ({}): odi key new {name}", p.display()))?;
    let mut k = [0u8; 32];
    hex::decode_to_slice(h.trim(), &mut k).map_err(|_| anyhow!("{}: not a key", p.display()))?;
    Ok(k)
}

/// odi key new [NAME] | odi key ls
pub fn cmd_key(args: &[String]) -> Result<()> {
    const U: &str = "odi key new [NAME] | odi key ls";
    match args.split_first() {
        Some((c, rest)) if c == "new" && rest.len() <= 1 => {
            let name = rest.first().map(String::as_str).unwrap_or("default");
            if !odi_format::valid_name(name) {
                bail!("key {name:?}: letters, digits, . - _");
            }
            let dir = keys_dir();
            fs::create_dir_all(&dir)?;
            crate::set_mode(&dir, 0o700)?;
            let p = dir.join(format!("{name}.key"));
            if p.exists() {
                bail!("{} exists already", p.display());
            }
            let mut k = [0u8; 32];
            getrandom::getrandom(&mut k).map_err(|e| anyhow!("no randomness here: {e}"))?;
            let tmp = dir.join(format!(".{name}.key.tmp"));
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                let mut f = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
                std::io::Write::write_all(&mut f, hex::encode(k).as_bytes())?;
            }
            #[cfg(not(unix))]
            fs::write(&tmp, hex::encode(k))?;
            fs::rename(&tmp, &p)?;
            println!("{}  ({name}, in {})", key_string(&public_key(&k)), p.display());
            println!(
                "Give the public key to whoever runs what you sign: odi run --trust KEY, or a line in ~/.odi/trust."
            );
            Ok(())
        }
        Some((c, rest)) if c == "ls" && rest.is_empty() => {
            let Ok(rd) = fs::read_dir(keys_dir()) else { return Ok(()) };
            let mut names: Vec<String> = rd
                .filter_map(|e| e.ok()?.file_name().into_string().ok()?.strip_suffix(".key").map(String::from))
                .collect();
            names.sort();
            for n in names {
                println!("{}  {n}", key_string(&public_key(&secret(&n)?)));
            }
            for k in trusted(&[])? {
                println!("{k}  (trusted)");
            }
            Ok(())
        }
        _ => bail!("usage: {U}"),
    }
}

/// odi sign FILE... [--key NAME]: adds a signature, keeping any others.
pub fn cmd_sign(args: &[String]) -> Result<()> {
    const U: &str = "odi sign FILE... [--key NAME]";
    let specs = [flag("key", Value, "the key to sign with (default: default)")];
    let (f, pos) = flags::parse(&specs, args, U)?;
    if pos.is_empty() {
        bail!("usage: {U}");
    }
    let k = secret(f.str("key").unwrap_or("default"))?;
    for file in &pos {
        let path = Path::new(file);
        let rd = Reader::open(path).with_context(|| file.clone())?;
        let new = sign(&k, &rd.digest);
        let mut sigs = Signatures { signatures: rd.signatures.clone() };
        sigs.signatures.retain(|s| s.key != new.key);
        sigs.signatures.push(new.clone());
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let r = (|| -> Result<()> {
            let mut out = std::io::BufWriter::new(fs::File::create(&tmp)?);
            write_signed(&rd, &sigs, &mut out)?;
            std::io::Write::flush(&mut out)?;
            out.into_inner().map_err(|e| anyhow!("{e}"))?.sync_all()?;
            Ok(())
        })();
        if let Err(e) = r {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        drop(rd);
        fs::rename(&tmp, path)?;
        // (Checked again, opened as a runner would.)
        let rd = Reader::open(path)?;
        if !rd.signatures.iter().any(|s| s == &new && s.check(&rd.digest).is_ok()) {
            bail!("{file}: the signature didn't come out right");
        }
        println!("{file} {}  signed by {}", digest_string(&rd.digest), new.key);
    }
    Ok(())
}

/// The keys this runner trusts: ~/.odi/trust, $ODI_TRUST and given ones.
pub fn trusted(given: &[String]) -> Result<Vec<String>> {
    let mut keys: Vec<String> = given.to_vec();
    if let Ok(t) = std::env::var("ODI_TRUST") {
        keys.extend(t.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from));
    }
    if let Ok(t) = fs::read_to_string(crate::hub::odi_home().join("trust")) {
        keys.extend(
            t.lines().map(|l| l.split('#').next().unwrap_or("").trim()).filter(|l| !l.is_empty()).map(String::from),
        );
    }
    for k in &keys {
        parse_key(k).map_err(|e| anyhow!("a trusted key: {e}"))?;
    }
    keys.sort();
    keys.dedup();
    Ok(keys)
}

/// With trusted keys, a file one of them signed, or an error saying why not.
/// Without any, everything runs (the digest still proves the contents).
pub fn check_trusted(rd: &Reader, given: &[String]) -> Result<()> {
    let keys = trusted(given)?;
    if keys.is_empty() {
        return Ok(());
    }
    for s in &rd.signatures {
        if keys.contains(&s.key) && s.check(&rd.digest).is_ok() {
            eprintln!("odi: signed by {} (trusted)", s.key);
            return Ok(());
        }
    }
    if rd.signatures.is_empty() {
        bail!("{} isn't signed, and this runner runs only what a trusted key signed", digest_string(&rd.digest));
    }
    bail!(
        "{} isn't signed by a key this runner trusts (signed by {})",
        digest_string(&rd.digest),
        rd.signatures.iter().map(|s| s.key.as_str()).collect::<Vec<_>>().join(", ")
    )
}
