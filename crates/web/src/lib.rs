//! ODI files in a browser (web/: the runner). The page fetches bytes (HTTP
//! Range requests); this checks them, as every reader must (SPEC §3, §6):
//!
//! - [`odi_open`]: the file's first 64 bytes and its last bytes (footer,
//!   manifest, index, signatures) to the digest-checked manifest and index.
//! - [`odi_chunk`]: a stored chunk to its bytes, checked against its hash.
//!
//! A C ABI, nothing else: buffers in from [`odi_alloc`], the answer (JSON,
//! bytes, or an error's words) in one output buffer ([`odi_out_ptr`],
//! [`odi_out_len`]).

use std::cell::RefCell;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};

use odi_format::{decode_chunk_checked, digest_string, Chunk, ReadAt, Reader};

thread_local! {
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn set_out(b: Vec<u8>) {
    OUT.with(|o| *o.borrow_mut() = b);
}

/// Room for n bytes, for the page to fill.
#[no_mangle]
pub extern "C" fn odi_alloc(n: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(n.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// Gives back what odi_alloc gave.
///
/// # Safety
/// p and n are what odi_alloc gave and was asked for.
#[no_mangle]
pub unsafe extern "C" fn odi_free(p: *mut u8, n: usize) {
    drop(Vec::from_raw_parts(p, 0, n.max(1)));
}

#[no_mangle]
pub extern "C" fn odi_out_ptr() -> *const u8 {
    OUT.with(|o| o.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn odi_out_len() -> usize {
    OUT.with(|o| o.borrow().len())
}

/// The bytes of a file the page has: its head, and its tail from an offset.
struct Known {
    size: u64,
    head: Vec<u8>,
    tail_at: u64,
    tail: Vec<u8>,
    /// The lowest offset asked for that isn't here.
    missing: AtomicU64,
}

impl ReadAt for Known {
    fn read_exact_at(&self, buf: &mut [u8], off: u64) -> io::Result<()> {
        let end = off + buf.len() as u64;
        if end <= self.head.len() as u64 {
            buf.copy_from_slice(&self.head[off as usize..end as usize]);
            return Ok(());
        }
        if off >= self.tail_at && end <= self.size {
            let at = (off - self.tail_at) as usize;
            buf.copy_from_slice(&self.tail[at..at + buf.len()]);
            return Ok(());
        }
        self.missing.fetch_min(off, Ordering::Relaxed);
        Err(io::Error::other("not fetched"))
    }
}

/// What a file says, from its head and tail. Returns 0 with the answer in
/// the output (JSON: digest, manifest, entries, signatures with whether each
/// checks), 2 when the tail doesn't reach far enough (output: the offset to
/// fetch from), 1 on an error (output: why).
///
/// # Safety
/// The pointers hold the lengths given.
#[no_mangle]
pub unsafe extern "C" fn odi_open(
    size: f64,
    head: *const u8,
    head_len: usize,
    tail: *const u8,
    tail_len: usize,
) -> i32 {
    let size = size as u64;
    let k = Known {
        size,
        head: std::slice::from_raw_parts(head, head_len).to_vec(),
        tail_at: size.saturating_sub(tail_len as u64),
        tail: std::slice::from_raw_parts(tail, tail_len).to_vec(),
        missing: AtomicU64::new(u64::MAX),
    };
    match open(k) {
        Ok(json) => {
            set_out(json);
            0
        }
        Err((Some(from), _)) => {
            set_out(from.to_string().into_bytes());
            2
        }
        Err((None, e)) => {
            set_out(e.into_bytes());
            1
        }
    }
}

type Failed = (Option<u64>, String);

fn open(k: Known) -> Result<Vec<u8>, Failed> {
    let size = k.size;
    let known = std::sync::Arc::new(k);
    let rd = match Reader::new(known.clone(), size) {
        Ok(rd) => rd,
        Err(e) => {
            let m = known.missing.load(Ordering::Relaxed);
            return Err(((m != u64::MAX).then_some(m), e.to_string()));
        }
    };
    let manifest: serde_json::Value = serde_json::from_slice(&rd.manifest_json).map_err(|e| (None, e.to_string()))?;
    let sigs: Vec<serde_json::Value> = rd
        .signatures
        .iter()
        .map(|s| serde_json::json!({ "key": s.key, "valid": s.check(&rd.digest).is_ok() }))
        .collect();
    let v = serde_json::json!({
        "digest": digest_string(&rd.digest),
        "manifest": manifest,
        "entries": rd.entries,
        "signatures": sigs,
    });
    serde_json::to_vec(&v).map_err(|e| (None, e.to_string()))
}

/// A stored chunk to its bytes, checked. chunk: its index entry as JSON
/// (hash, offset, length, size). Returns 0 with the bytes in the output, or 1
/// with why not.
///
/// # Safety
/// The pointers hold the lengths given.
#[no_mangle]
pub unsafe extern "C" fn odi_chunk(stored: *const u8, len: usize, chunk: *const u8, chunk_len: usize) -> i32 {
    let c: Chunk = match serde_json::from_slice(std::slice::from_raw_parts(chunk, chunk_len)) {
        Ok(c) => c,
        Err(e) => {
            set_out(e.to_string().into_bytes());
            return 1;
        }
    };
    match decode_chunk_checked(std::slice::from_raw_parts(stored, len).to_vec(), &c) {
        Ok(b) => {
            set_out(b);
            0
        }
        Err(e) => {
            set_out(e.to_string().into_bytes());
            1
        }
    }
}

#[cfg(test)]
mod tests;
