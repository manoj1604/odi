//! The ODI digest (§6) and the canonical JSON it hashes.
//!
//! Strings are written exactly as Go's encoding/json writes them with HTML
//! escaping off, since that's what files already published were digested
//! with: `"` and `\` escaped, `\b \f \n \r \t` short, other control
//! characters as `\u00xx`, and U+2028 and U+2029 as ` ` and ` `.

use sha2::{Digest as _, Sha256};

use crate::{Entry, Error, Result};

/// JSON as canonical JSON (§6): object keys sorted by their bytes, no
/// insignificant whitespace, strings with only the escapes JSON requires
/// (`<`, `>` and `&` as they are), and numbers exactly as written. It reads
/// strings as Go's decoder does (a lone surrogate or a byte that isn't UTF-8
/// is U+FFFD; of two equal keys the last counts), so it digests what the Go
/// implementation digested.
pub fn canonical(json: &[u8]) -> Result<Vec<u8>> {
    let mut p = Parser { b: json, i: 0 };
    let mut out = Vec::with_capacity(json.len());
    p.ws();
    p.value(&mut out, 0)?;
    p.ws();
    if p.i != json.len() {
        return Err(p.err("after the top-level value"));
    }
    Ok(out)
}

/// Deeper than this is refused (Go allows 10000; no manifest comes close).
const MAX_DEPTH: usize = 1000;

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err(&self, what: &str) -> Error {
        Error(format!("JSON: invalid at byte {} ({what})", self.i))
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> Result<()> {
        if self.peek() != Some(c) {
            return Err(self.err(&format!("expected {:?}", c as char)));
        }
        self.i += 1;
        Ok(())
    }

    fn value(&mut self, out: &mut Vec<u8>, depth: usize) -> Result<()> {
        if depth > MAX_DEPTH {
            return Err(self.err("nested too deep"));
        }
        match self.peek() {
            Some(b'{') => {
                self.i += 1;
                let mut members: Vec<(String, Vec<u8>)> = Vec::new();
                self.ws();
                if self.peek() == Some(b'}') {
                    self.i += 1;
                } else {
                    loop {
                        self.ws();
                        let k = self.string()?;
                        self.ws();
                        self.eat(b':')?;
                        self.ws();
                        let mut v = Vec::new();
                        self.value(&mut v, depth + 1)?;
                        members.push((k, v));
                        self.ws();
                        match self.peek() {
                            Some(b',') => self.i += 1,
                            Some(b'}') => {
                                self.i += 1;
                                break;
                            }
                            _ => return Err(self.err("expected , or }")),
                        }
                    }
                }
                // (Stable, so of equal keys the last one written is last.)
                members.sort_by(|a, b| a.0.cmp(&b.0));
                out.push(b'{');
                let mut first = true;
                for (n, (k, v)) in members.iter().enumerate() {
                    if members.get(n + 1).is_some_and(|next| next.0 == *k) {
                        continue;
                    }
                    if !std::mem::take(&mut first) {
                        out.push(b',');
                    }
                    write_string(out, k);
                    out.push(b':');
                    out.extend_from_slice(v);
                }
                out.push(b'}');
            }
            Some(b'[') => {
                self.i += 1;
                out.push(b'[');
                self.ws();
                if self.peek() == Some(b']') {
                    self.i += 1;
                } else {
                    let mut first = true;
                    loop {
                        if !std::mem::take(&mut first) {
                            out.push(b',');
                        }
                        self.ws();
                        self.value(out, depth + 1)?;
                        self.ws();
                        match self.peek() {
                            Some(b',') => self.i += 1,
                            Some(b']') => {
                                self.i += 1;
                                break;
                            }
                            _ => return Err(self.err("expected , or ]")),
                        }
                    }
                }
                out.push(b']');
            }
            Some(b'"') => {
                let s = self.string()?;
                write_string(out, &s);
            }
            Some(b't') => self.word(out, b"true")?,
            Some(b'f') => self.word(out, b"false")?,
            Some(b'n') => self.word(out, b"null")?,
            Some(b'-' | b'0'..=b'9') => self.number(out)?,
            _ => return Err(self.err("expected a value")),
        }
        Ok(())
    }

    fn word(&mut self, out: &mut Vec<u8>, w: &[u8]) -> Result<()> {
        if !self.b[self.i..].starts_with(w) {
            return Err(self.err("expected a value"));
        }
        self.i += w.len();
        out.extend_from_slice(w);
        Ok(())
    }

    /// -? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?, kept as written.
    fn number(&mut self, out: &mut Vec<u8>) -> Result<()> {
        let start = self.i;
        let digits = |p: &mut Self| -> usize {
            let s = p.i;
            while matches!(p.peek(), Some(b'0'..=b'9')) {
                p.i += 1;
            }
            p.i - s
        };
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        match self.peek() {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                digits(self);
            }
            _ => return Err(self.err("a number")),
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            if digits(self) == 0 {
                return Err(self.err("a number"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if digits(self) == 0 {
                return Err(self.err("a number"));
            }
        }
        out.extend_from_slice(&self.b[start..self.i]);
        Ok(())
    }

    fn hex4(&self, at: usize) -> Option<u32> {
        let h = self.b.get(at..at + 4)?;
        let s = std::str::from_utf8(h).ok()?;
        if !s.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(s, 16).ok()
    }

    fn string(&mut self) -> Result<String> {
        self.eat(b'"')?;
        let mut s = String::new();
        loop {
            let Some(c) = self.peek() else { return Err(self.err("an unfinished string")) };
            match c {
                b'"' => {
                    self.i += 1;
                    return Ok(s);
                }
                b'\\' => {
                    let Some(e) = self.b.get(self.i + 1).copied() else { return Err(self.err("an escape")) };
                    self.i += 2;
                    match e {
                        b'"' => s.push('"'),
                        b'\\' => s.push('\\'),
                        b'/' => s.push('/'),
                        b'b' => s.push('\u{8}'),
                        b'f' => s.push('\u{c}'),
                        b'n' => s.push('\n'),
                        b'r' => s.push('\r'),
                        b't' => s.push('\t'),
                        b'u' => {
                            let Some(r) = self.hex4(self.i) else { return Err(self.err("an escape")) };
                            self.i += 4;
                            let mut ch = char::from_u32(r);
                            if (0xd800..0xe000).contains(&r) {
                                ch = None;
                                // (A pair makes one character; a lone half is U+FFFD.)
                                if (0xd800..0xdc00).contains(&r) && self.b.get(self.i..self.i + 2) == Some(b"\\u") {
                                    if let Some(lo) = self.hex4(self.i + 2).filter(|lo| (0xdc00..0xe000).contains(lo)) {
                                        self.i += 6;
                                        ch = char::from_u32(0x10000 + ((r - 0xd800) << 10) + (lo - 0xdc00));
                                    }
                                }
                            }
                            s.push(ch.unwrap_or('\u{fffd}'));
                        }
                        _ => return Err(self.err("an escape")),
                    }
                }
                0..=0x1f => return Err(self.err("a control character in a string")),
                0x20..=0x7f => {
                    s.push(c as char);
                    self.i += 1;
                }
                _ => {
                    // (UTF-8 is prefix-free: the first prefix that decodes is the
                    // character. None does: one byte, U+FFFD, as Go.)
                    let rest = &self.b[self.i..];
                    match (1..=rest.len().min(4)).find_map(|n| std::str::from_utf8(&rest[..n]).ok()) {
                        Some(ch) => {
                            s.push_str(ch);
                            self.i += ch.len();
                        }
                        None => {
                            s.push('\u{fffd}');
                            self.i += 1;
                        }
                    }
                }
            }
        }
    }
}

pub(crate) fn write_string(b: &mut Vec<u8>, s: &str) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    b.push(b'"');
    for c in s.chars() {
        match c {
            '"' => b.extend_from_slice(b"\\\""),
            '\\' => b.extend_from_slice(b"\\\\"),
            '\u{8}' => b.extend_from_slice(b"\\b"),
            '\u{c}' => b.extend_from_slice(b"\\f"),
            '\n' => b.extend_from_slice(b"\\n"),
            '\r' => b.extend_from_slice(b"\\r"),
            '\t' => b.extend_from_slice(b"\\t"),
            c if (c as u32) < 0x20 => {
                let n = c as usize;
                b.extend_from_slice(b"\\u00");
                b.push(HEX[n >> 4]);
                b.push(HEX[n & 15]);
            }
            '\u{2028}' => b.extend_from_slice(b"\\u2028"),
            '\u{2029}' => b.extend_from_slice(b"\\u2029"),
            c => {
                let mut buf = [0u8; 4];
                b.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    b.push(b'"');
}

/// The entries without where their chunks are (layout, not content), keys
/// in order: chunks, mode, path, size, target, type; chunks: hash, size.
fn canonical_entries(entries: &[Entry]) -> Vec<u8> {
    let mut b = Vec::with_capacity(entries.len() * 128);
    b.push(b'[');
    for (i, e) in entries.iter().enumerate() {
        if i > 0 {
            b.push(b',');
        }
        let mut first = true;
        let mut key = |b: &mut Vec<u8>, k: &str| {
            if !std::mem::take(&mut first) {
                b.push(b',');
            }
            write_string(b, k);
            b.push(b':');
        };
        b.push(b'{');
        if !e.chunks.is_empty() {
            key(&mut b, "chunks");
            b.push(b'[');
            for (j, c) in e.chunks.iter().enumerate() {
                if j > 0 {
                    b.push(b',');
                }
                b.extend_from_slice(b"{\"hash\":");
                write_string(&mut b, &c.hash);
                b.extend_from_slice(format!(",\"size\":{}}}", c.size).as_bytes());
            }
            b.push(b']');
        }
        if e.mode != 0 {
            key(&mut b, "mode");
            b.extend_from_slice(e.mode.to_string().as_bytes());
        }
        key(&mut b, "path");
        write_string(&mut b, &e.path);
        if e.size != 0 {
            key(&mut b, "size");
            b.extend_from_slice(e.size.to_string().as_bytes());
        }
        if !e.target.is_empty() {
            key(&mut b, "target");
            write_string(&mut b, &e.target);
        }
        key(&mut b, "type");
        write_string(&mut b, e.kind.as_str());
        b.push(b'}');
    }
    b.push(b']');
    b
}

/// The ODI digest (§6) of a manifest (as its JSON, so fields this reader
/// doesn't know count) and the entries.
pub fn digest(manifest_json: &[u8], entries: &[Entry]) -> Result<[u8; 32]> {
    let m = canonical(manifest_json).map_err(|e| Error(format!("manifest: {e}")))?;
    let mut h = Sha256::new();
    h.update(b"odi-v0\n");
    h.update(m);
    h.update(b"\n");
    h.update(canonical_entries(entries));
    Ok(h.finalize().into())
}

/// A digest written as `sha256:HEX`.
pub fn digest_string(d: &[u8; 32]) -> String {
    format!("sha256:{}", hex::encode(d))
}
