use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::sync::Arc;

use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use crate::{bail, check_entry, check_tree, digest, Chunk, Entry, Error, Kind, Manifest, Result};
use crate::{COMPRESS_SLACK, FOOTER_SIZE, HEADER_SIZE, MAGIC, MAJOR, MAX_CHUNK, MAX_ENTRIES, MAX_INDEX, MAX_MANIFEST};

/// Anything that reads at offsets: a file on disk, bytes in memory, or a
/// remote file over HTTP range requests.
pub trait ReadAt: Send + Sync {
    /// Fills buf from off, or fails.
    fn read_exact_at(&self, buf: &mut [u8], off: u64) -> io::Result<()>;
}

#[cfg(any(unix, windows))]
impl ReadAt for std::fs::File {
    #[cfg(unix)]
    fn read_exact_at(&self, buf: &mut [u8], off: u64) -> io::Result<()> {
        std::os::unix::fs::FileExt::read_exact_at(self, buf, off)
    }

    #[cfg(windows)]
    fn read_exact_at(&self, mut buf: &mut [u8], mut off: u64) -> io::Result<()> {
        use std::os::windows::fs::FileExt;
        while !buf.is_empty() {
            match self.seek_read(buf, off) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => {
                    buf = &mut buf[n..];
                    off += n as u64;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

impl ReadAt for Vec<u8> {
    fn read_exact_at(&self, buf: &mut [u8], off: u64) -> io::Result<()> {
        let start = usize::try_from(off).map_err(|_| io::Error::from(io::ErrorKind::UnexpectedEof))?;
        match start.checked_add(buf.len()).and_then(|end| self.get(start..end)) {
            Some(src) => {
                buf.copy_from_slice(src);
                Ok(())
            }
            None => Err(io::ErrorKind::UnexpectedEof.into()),
        }
    }
}

impl<T: ReadAt + ?Sized> ReadAt for Arc<T> {
    fn read_exact_at(&self, buf: &mut [u8], off: u64) -> io::Result<()> {
        (**self).read_exact_at(buf, off)
    }
}

/// Reads an ODI file from anything that reads at offsets (§3). Every check a
/// reader must make is made when it's opened; chunks are checked as read.
pub struct Reader {
    r: Box<dyn ReadAt>,
    size: u64,
    pub manifest: Manifest,
    pub manifest_json: Vec<u8>,
    pub entries: Vec<Entry>,
    pub digest: [u8; 32],
    /// Its signatures (§3.5), not checked: [`crate::Signature::check`] them
    /// against keys the runner trusts.
    pub signatures: Vec<crate::Signature>,
    by_path: HashMap<String, usize>,
}

/// Decompresses a zstd stream, refusing more than limit bytes.
#[cfg(feature = "zstd")]
fn decode_limited(data: &[u8], limit: u64) -> Result<Vec<u8>> {
    let mut dec = zstd::stream::read::Decoder::with_buffer(data)?;
    dec.window_log_max(27)?;
    let mut out = Vec::new();
    dec.take(limit + 1).read_to_end(&mut out)?;
    if out.len() as u64 > limit {
        bail!("too big");
    }
    Ok(out)
}

#[cfg(not(feature = "zstd"))]
fn decode_limited(mut data: &[u8], limit: u64) -> Result<Vec<u8>> {
    let dec = ruzstd::decoding::StreamingDecoder::new(&mut data).map_err(|e| Error(format!("zstd: {e}")))?;
    let mut out = Vec::new();
    dec.take(limit + 1).read_to_end(&mut out)?;
    if out.len() as u64 > limit {
        bail!("too big");
    }
    Ok(out)
}

#[cfg(feature = "zstd")]
thread_local! {
    static CHUNKS: std::cell::RefCell<Option<zstd::bulk::Decompressor<'static>>> = const { std::cell::RefCell::new(None) };
}

/// Decompresses a chunk into exactly size bytes, never more.
#[cfg(feature = "zstd")]
fn decode_chunk(data: &[u8], size: usize) -> io::Result<Vec<u8>> {
    CHUNKS.with(|d| {
        let mut d = d.borrow_mut();
        if d.is_none() {
            *d = Some(zstd::bulk::Decompressor::new()?);
        }
        d.as_mut().expect("just made").decompress(data, size)
    })
}

#[cfg(not(feature = "zstd"))]
fn decode_chunk(data: &[u8], size: usize) -> io::Result<Vec<u8>> {
    decode_limited(data, size as u64).map_err(|e| io::Error::other(e.0))
}

/// A stored chunk (§3.3), as read from the file, decompressed and checked
/// against its hash and size: for readers that fetch chunks themselves (the
/// browser runner).
pub fn decode_chunk_checked(mut b: Vec<u8>, c: &Chunk) -> Result<Vec<u8>> {
    if b.len() as u64 != c.length
        || c.length < 4
        || c.size > MAX_CHUNK
        || c.length > c.size + c.size / 8 + COMPRESS_SLACK
    {
        bail!("chunk {}: bad length", c.hash);
    }
    let out = match b[0] {
        0 => {
            b.drain(..4);
            b
        }
        1 => decode_chunk(&b[4..], c.size as usize).map_err(|e| Error(format!("chunk {}: {e}", c.hash)))?,
        n => bail!("chunk {}: encoding {}", c.hash, n),
    };
    if out.len() as u64 != c.size || hex::encode(Sha256::digest(&out)) != c.hash {
        bail!("chunk {}: damaged", c.hash);
    }
    Ok(out)
}

impl Reader {
    /// Opens an ODI file on disk.
    #[cfg(any(unix, windows))]
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Reader> {
        let f = std::fs::File::open(path)?;
        let size = f.metadata()?.len();
        Reader::new(f, size)
    }

    /// Reads the header, footer, manifest and index, and checks them and the
    /// digest (§3.2, §6).
    pub fn new<R: ReadAt + 'static>(r: R, size: u64) -> Result<Reader> {
        if size < HEADER_SIZE + FOOTER_SIZE {
            bail!("not an ODI file (too short)");
        }
        let mut h = [0u8; HEADER_SIZE as usize];
        r.read_exact_at(&mut h, 0)?;
        if &h[..8] != MAGIC {
            bail!("not an ODI file");
        }
        let major = u16::from_le_bytes([h[8], h[9]]);
        if major != MAJOR {
            bail!("ODI format {}.x: this reader knows {}.x", major, MAJOR);
        }
        let mut f = [0u8; FOOTER_SIZE as usize];
        r.read_exact_at(&mut f, size - FOOTER_SIZE)?;
        if &f[120..] != MAGIC {
            bail!("not an ODI file (no footer)");
        }
        let field = |i: usize| u64::from_le_bytes(f[i..i + 8].try_into().expect("8 bytes"));
        let (m_off, m_len, i_off, i_len) = (field(0), field(8), field(16), field(24));
        let end = size - FOOTER_SIZE;
        let within = move |off: u64, n: u64| off >= HEADER_SIZE && off.checked_add(n).is_some_and(|e| e <= end);
        if !within(m_off, m_len) || !within(i_off, i_len) || (field(32) != 0 && !within(field(32), field(40))) {
            bail!("ODI file: an offset outside the file");
        }
        if m_len > MAX_MANIFEST + COMPRESS_SLACK || i_len > MAX_INDEX + COMPRESS_SLACK {
            bail!("ODI file: a manifest or index too big");
        }
        let read = |off: u64, n: u64, limit: u64| -> Result<Vec<u8>> {
            let mut b = vec![0u8; n as usize];
            r.read_exact_at(&mut b, off)?;
            decode_limited(&b, limit)
        };
        let manifest_json = read(m_off, m_len, MAX_MANIFEST).map_err(|e| Error(format!("manifest: {e}")))?;
        let manifest: Manifest = serde_json::from_slice(&manifest_json).map_err(|e| Error(format!("manifest: {e}")))?;
        manifest.check()?;
        let ij = read(i_off, i_len, MAX_INDEX).map_err(|e| Error(format!("index: {e}")))?;
        #[derive(Deserialize)]
        struct Index {
            #[serde(default)]
            entries: Vec<Entry>,
        }
        let idx: Index = serde_json::from_slice(&ij).map_err(|e| Error(format!("index: {e}")))?;
        drop(ij);
        if idx.entries.len() > MAX_ENTRIES {
            bail!("index: too many entries");
        }
        let mut by_path = HashMap::with_capacity(idx.entries.len());
        for (i, e) in idx.entries.iter().enumerate() {
            check_entry(e)?;
            if i > 0 && idx.entries[i - 1].path >= e.path {
                bail!("index: entries not sorted, or a path twice");
            }
            for c in &e.chunks {
                if !within(c.offset, c.length) || c.length < 4 || c.length > c.size + c.size / 8 + COMPRESS_SLACK {
                    bail!("{}: a chunk outside the file", e.path);
                }
            }
            by_path.insert(e.path.clone(), i);
        }
        check_tree(&manifest, &idx.entries)?;
        let d = digest(&manifest_json, &idx.entries)?;
        if d[..] != f[48..80] {
            bail!("ODI file: the digest doesn't match its contents");
        }
        let mut signatures = Vec::new();
        if field(32) != 0 {
            if field(40) > crate::sign::MAX_SIGNATURES + COMPRESS_SLACK {
                bail!("ODI file: a signatures block too big");
            }
            let sj = read(field(32), field(40), crate::sign::MAX_SIGNATURES)
                .map_err(|e| Error(format!("signatures: {e}")))?;
            let sigs: crate::Signatures = serde_json::from_slice(&sj).map_err(|e| Error(format!("signatures: {e}")))?;
            signatures = sigs.signatures;
        }
        Ok(Reader {
            r: Box::new(r),
            size,
            manifest,
            manifest_json,
            entries: idx.entries,
            digest: d,
            signatures,
            by_path,
        })
    }

    /// A reader over a tree put together from readers already opened (a set
    /// and its parts, say): `entries` sorted by path, each chunk's offset in
    /// `r`. The tree is checked again as a whole (links that cross from one
    /// part to another must stay inside it); `digest` is what it's known by
    /// (the set's).
    pub fn assembled<R: ReadAt + 'static>(
        r: R,
        size: u64,
        manifest: Manifest,
        manifest_json: Vec<u8>,
        entries: Vec<Entry>,
        digest: [u8; 32],
    ) -> Result<Reader> {
        let mut by_path = HashMap::with_capacity(entries.len());
        for (i, e) in entries.iter().enumerate() {
            crate::check_path(&e.path)?;
            if i > 0 && entries[i - 1].path >= e.path {
                bail!("assembled tree: entries not sorted, or a path twice");
            }
            for c in &e.chunks {
                if c.offset.checked_add(c.length).is_none_or(|end| end > size) {
                    bail!("{}: a chunk outside the parts", e.path);
                }
            }
            by_path.insert(e.path.clone(), i);
        }
        check_tree(&manifest, &entries)?;
        Ok(Reader { r: Box::new(r), size, manifest, manifest_json, entries, digest, signatures: Vec::new(), by_path })
    }

    /// A reader over entries that were never checked, holding no content: for
    /// testing that what reads it holds up by itself against a hostile index.
    #[doc(hidden)]
    pub fn from_entries_unchecked(entries: Vec<Entry>) -> Reader {
        let by_path = entries.iter().enumerate().map(|(i, e)| (e.path.clone(), i)).collect();
        Reader {
            r: Box::new(Vec::new()),
            size: 0,
            manifest: Manifest::default(),
            manifest_json: Vec::new(),
            entries,
            digest: [0; 32],
            signatures: Vec::new(),
            by_path,
        }
    }

    /// The file's length in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The file's raw bytes at off (not checked: chunks are checked by
    /// [`Reader::read_chunk`]).
    pub fn read_at(&self, buf: &mut [u8], off: u64) -> io::Result<()> {
        self.r.read_exact_at(buf, off)
    }

    /// An entry's place in `entries`, by path.
    pub fn entry_index(&self, path: &str) -> Option<usize> {
        self.by_path.get(path).copied()
    }

    /// Finds an entry by path.
    pub fn entry(&self, path: &str) -> Option<&Entry> {
        self.by_path.get(path).map(|&i| &self.entries[i])
    }

    /// Reads one chunk, decompresses it and checks its hash.
    pub fn read_chunk(&self, c: &Chunk) -> Result<Vec<u8>> {
        if c.length < 4 || c.size > MAX_CHUNK || c.length > c.size + c.size / 8 + COMPRESS_SLACK {
            bail!("chunk {}: bad length", c.hash);
        }
        let mut b = vec![0u8; c.length as usize];
        self.r.read_exact_at(&mut b, c.offset)?;
        decode_chunk_checked(b, c)
    }

    /// Writes a file entry's content to w, every chunk checked.
    pub fn write_file(&self, e: &Entry, w: &mut dyn Write) -> Result<()> {
        if e.kind != Kind::File {
            bail!("{} isn't a file", e.path);
        }
        for c in &e.chunks {
            let b = self.read_chunk(c).map_err(|err| Error(format!("{}: {err}", e.path)))?;
            w.write_all(&b)?;
        }
        Ok(())
    }

    /// A file entry's content.
    pub fn read_file(&self, path: &str) -> Result<Vec<u8>> {
        let Some(e) = self.entry(path) else { bail!("{}: not in the file", path) };
        let mut b = Vec::with_capacity(e.size.min(1 << 30) as usize);
        self.write_file(e, &mut b)?;
        Ok(b)
    }

    /// Reads every chunk once and checks it.
    pub fn verify(&self) -> Result<()> {
        let mut done = HashSet::new();
        for e in &self.entries {
            for c in &e.chunks {
                if done.contains(&c.hash) {
                    continue;
                }
                self.read_chunk(c).map_err(|err| Error(format!("{}: {err}", e.path)))?;
                done.insert(c.hash.clone());
            }
        }
        Ok(())
    }
}
