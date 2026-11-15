//! Content-defined chunking (§3.3: SHOULD): a gear rolling hash cuts where
//! the content says, between 512 KiB and 4 MiB, about 1 MiB on average, so a
//! small change in a big file changes few chunks. (Readers don't depend on
//! these numbers: any cuts are valid.)

use std::io::{ErrorKind, Read};
use std::sync::OnceLock;

use crate::{Result, MAX_CHUNK};

/// The smallest chunk, and the mask whose zero hash cuts. ($ODI_CHUNK, an
/// experiment: the average size in bytes, a power of two.)
fn params() -> (usize, u64) {
    static P: OnceLock<(usize, u64)> = OnceLock::new();
    *P.get_or_init(|| match std::env::var("ODI_CHUNK").ok().and_then(|s| s.parse::<u64>().ok()) {
        Some(n) if n > 0 => ((n / 2) as usize, n - 1),
        _ => (512 << 10, (1 << 20) - 1),
    })
}

/// Fixed (seeded with "odi"), so builds are reproducible.
const GEAR: [u64; 256] = {
    let mut g = [0u64; 256];
    let mut x: u64 = 0x6f6469;
    let mut i = 0;
    while i < 256 {
        x = x.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        g[i] = z ^ (z >> 31);
        i += 1;
    }
    g
};

/// Reads r to the end and calls emit with each chunk (the slice is reused:
/// emit must not keep it). An empty file has no chunks.
pub fn split<R: Read>(mut r: R, mut emit: impl FnMut(&[u8]) -> Result<()>) -> Result<()> {
    let (min, mask) = params();
    let max = MAX_CHUNK as usize;
    let mut input = vec![0u8; 1 << 20];
    let mut buf: Vec<u8> = Vec::with_capacity(max);
    let mut h: u64 = 0;
    loop {
        let n = match r.read(&mut input) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        };
        for &b in &input[..n] {
            buf.push(b);
            h = (h << 1).wrapping_add(GEAR[b as usize]);
            let len = buf.len();
            if len >= max || (len >= min && h & mask == 0) {
                emit(&buf)?;
                buf.clear();
                h = 0;
            }
        }
    }
    if !buf.is_empty() {
        emit(&buf)?;
    }
    Ok(())
}
