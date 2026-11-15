use std::collections::HashMap;
use std::io::{BufWriter, Read, Write};

use serde::Serialize;
use sha2::{Digest as _, Sha256};

use crate::{bail, check_entry, check_tree, digest, split, Chunk, Entry, Kind, Manifest, Result};
use crate::{FOOTER_SIZE, HEADER_SIZE, MAGIC, MAJOR, MINOR};

/// zstd's level for chunks, the manifest and the index: about what the Go
/// implementation used (SpeedBetterCompression).
const LEVEL: i32 = 7;

/// Makes an ODI file in one pass: chunks as entries are added, then the
/// manifest, the index and the footer (§3).
pub struct Writer<W: Write> {
    w: BufWriter<W>,
    off: u64,
    entries: Vec<Entry>,
    /// A chunk stored once, referenced from each place.
    seen: HashMap<String, Chunk>,
    zstd: zstd::bulk::Compressor<'static>,
}

impl<W: Write> Writer<W> {
    pub fn new(w: W) -> Result<Self> {
        let mut ow = Writer {
            w: BufWriter::with_capacity(1 << 20, w),
            off: 0,
            entries: Vec::new(),
            seen: HashMap::new(),
            zstd: zstd::bulk::Compressor::new(LEVEL)?,
        };
        let mut h = [0u8; HEADER_SIZE as usize];
        h[..8].copy_from_slice(MAGIC);
        h[8..10].copy_from_slice(&MAJOR.to_le_bytes());
        h[10..12].copy_from_slice(&MINOR.to_le_bytes());
        ow.write(&h)?;
        Ok(ow)
    }

    /// zstd's level for what's written from now on (1 to 22; default 7). The
    /// digest doesn't depend on it: it's of the content, not how it's stored.
    pub fn set_level(&mut self, level: i32) -> Result<()> {
        self.zstd = zstd::bulk::Compressor::new(level)?;
        Ok(())
    }

    fn write(&mut self, b: &[u8]) -> Result<()> {
        self.w.write_all(b)?;
        self.off += b.len() as u64;
        Ok(())
    }

    pub fn add_dir(&mut self, path: &str, mode: u32) -> Result<()> {
        let mut e = Entry::new(path, Kind::Dir);
        e.mode = mode & 0o777;
        self.add(e)
    }

    pub fn add_symlink(&mut self, path: &str, target: &str) -> Result<()> {
        let mut e = Entry::new(path, Kind::Symlink);
        e.target = target.to_string();
        self.add(e)
    }

    /// Stores a file's content, cut at content-defined points.
    pub fn add_file<R: Read>(&mut self, path: &str, mode: u32, r: R) -> Result<()> {
        let mut e = Entry::new(path, Kind::File);
        e.mode = mode & 0o777;
        let mut chunks = Vec::new();
        split(r, |c| {
            let h = hex::encode(Sha256::digest(c));
            let ch = match self.seen.get(&h) {
                Some(ch) => ch.clone(),
                None => {
                    let z = self.zstd.compress(c)?;
                    let mut stored = Vec::with_capacity(4 + c.len().min(z.len()));
                    if z.len() < c.len() {
                        stored.extend_from_slice(&[1, 0, 0, 0]);
                        stored.extend_from_slice(&z);
                    } else {
                        stored.extend_from_slice(&[0, 0, 0, 0]);
                        stored.extend_from_slice(c);
                    }
                    let ch =
                        Chunk { hash: h.clone(), offset: self.off, length: stored.len() as u64, size: c.len() as u64 };
                    self.write(&stored)?;
                    self.seen.insert(h, ch.clone());
                    ch
                }
            };
            chunks.push(ch);
            Ok(())
        })
        .map_err(|err| crate::Error(format!("{path}: {err}")))?;
        e.size = chunks.iter().map(|c| c.size).sum();
        e.chunks = chunks;
        self.add(e)
    }

    fn add(&mut self, e: Entry) -> Result<()> {
        check_entry(&e)?;
        self.entries.push(e);
        Ok(())
    }

    /// Writes the manifest, the index and the footer, and returns the file's
    /// digest.
    pub fn finish(mut self, m: &mut Manifest) -> Result<[u8; 32]> {
        if m.odi.is_empty() {
            m.odi = "0.1".into();
        }
        m.check()?;
        self.entries.sort_by(|a, b| a.path.cmp(&b.path));
        for w in self.entries.windows(2) {
            if w[0].path == w[1].path {
                bail!("{}: twice", w[1].path);
            }
        }
        check_tree(m, &self.entries)?;
        let mj = serde_json::to_vec(m)?;
        let d = digest(&mj, &self.entries)?;
        #[derive(Serialize)]
        struct Index<'a> {
            entries: &'a [Entry],
        }
        let ij = serde_json::to_vec(&Index { entries: &self.entries })?;

        let mut foot = [0u8; FOOTER_SIZE as usize];
        let mz = self.zstd.compress(&mj)?;
        foot[0..8].copy_from_slice(&self.off.to_le_bytes());
        foot[8..16].copy_from_slice(&(mz.len() as u64).to_le_bytes());
        self.write(&mz)?;
        let iz = self.zstd.compress(&ij)?;
        foot[16..24].copy_from_slice(&self.off.to_le_bytes());
        foot[24..32].copy_from_slice(&(iz.len() as u64).to_le_bytes());
        self.write(&iz)?;
        foot[48..80].copy_from_slice(&d);
        foot[120..128].copy_from_slice(MAGIC);
        self.write(&foot)?;
        self.w.flush()?;
        Ok(d)
    }
}
