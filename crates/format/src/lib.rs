//! Reads and writes ODI files (SPEC.md, version 0.1): one file holding a
//! program's build output, a manifest saying how to run it, and an index of
//! its entries, readable without unpacking.

mod canonical;
mod chunk;
mod read;
mod sign;
#[cfg(feature = "zstd")]
mod write;

#[cfg(not(any(feature = "zstd", feature = "pure")))]
compile_error!("odi-format needs a zstd: the zstd feature (default) or pure");

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use serde::{Deserialize, Serialize};

pub use canonical::{canonical, digest, digest_string};
pub use chunk::split;
pub use read::decode_chunk_checked;
pub use read::{ReadAt, Reader};
#[cfg(feature = "zstd")]
pub use sign::write_signed;
pub use sign::{key_string, parse_key, public_key, sign, Signature, Signatures, SIGNED_PREFIX};
#[cfg(feature = "zstd")]
pub use write::Writer;

pub const MAGIC: &[u8; 8] = b"ODIFILE\0";
pub const HEADER_SIZE: u64 = 64;
pub const FOOTER_SIZE: u64 = 128;
pub const MAJOR: u16 = 0;
pub const MINOR: u16 = 1;
pub const MEDIA_TYPE: &str = "application/vnd.odi.v0";

/// The largest chunk (uncompressed), §3.3.
pub const MAX_CHUNK: u64 = 4 << 20;

// Limits a reader enforces (§9).
const MAX_MANIFEST: u64 = 1 << 20;
const MAX_INDEX: u64 = 256 << 20;
const MAX_ENTRIES: usize = 2_000_000;
/// How much bigger than its content stored data may be.
const COMPRESS_SLACK: u64 = 64 << 10;
/// How many symlinks one path may pass through (as Linux).
const MAX_LINKS: usize = 40;

/// What went wrong reading or writing an ODI file: a sentence for a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error(e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

macro_rules! bail {
    ($($t:tt)*) => { return Err($crate::Error(format!($($t)*))) };
}
pub(crate) use bail;

/// What an entry is (§3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    File,
    Dir,
    Symlink,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::File => "file",
            Kind::Dir => "dir",
            Kind::Symlink => "symlink",
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

fn is_zero<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

/// One file, directory or symlink in an ODI file (§3.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    #[serde(rename = "type")]
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub mode: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub size: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chunks: Vec<Chunk>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target: String,
}

impl Entry {
    pub fn new(path: impl Into<String>, kind: Kind) -> Self {
        Entry { path: path.into(), kind, mode: 0, size: 0, chunks: Vec::new(), target: String::new() }
    }
}

/// A piece of a file's content: `offset` and `length` locate the stored
/// chunk (§3.3) in the file; `hash` and `size` are of the uncompressed bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chunk {
    pub hash: String,
    pub offset: u64,
    pub length: u64,
    pub size: u64,
}

/// What the manifest says (§5). Fields this reader doesn't know are kept in
/// the reader's `manifest_json`, and are part of the digest.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub odi: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<Platform>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<Language>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<Runtime>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub processes: Vec<Process>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub r#static: Vec<Static>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writable: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Env>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<Resources>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<Build>,
    /// 0.2: the parts this file is made of, each pinned by digest (a set).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<Part>,
    /// 0.2: what this file gives the parts it's put with (a runtime, say).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provides: Vec<Plug>,
    /// 0.2: what it needs from them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<Plug>,
}

/// One part of a set (0.2): another ODI file, named by its digest.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Part {
    #[serde(default)]
    pub name: String,
    /// `sha256:` and the part's ODI digest.
    #[serde(default)]
    pub digest: String,
    /// How its files join the set: `main`, `runtime` and `overlay` are
    /// merged into the set's tree in the order listed (a later file replaces
    /// an earlier one); with no role, they're under `parts/NAME/`.
    #[serde(default, rename = "as", skip_serializing_if = "String::is_empty")]
    pub role: String,
    /// Where to get it when it isn't here: a name in a registry. Only a hint:
    /// whatever comes from there must have the digest.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub from: String,
}

/// The roles a part may have.
pub const ROLES: [&str; 3] = ["main", "runtime", "overlay"];

/// How deep sets may hold sets.
pub const MAX_DEPTH: usize = 8;

/// A plug (0.2): something a part provides or needs, `runtime` `node` `22.11.0`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Plug {
    #[serde(default)]
    pub plug: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    /// Which of the part's processes serves it (default: the first).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub process: String,
}

impl Part {
    /// The part's digest as 32 bytes, if it's written right.
    pub fn digest_bytes(&self) -> Option<[u8; 32]> {
        let h = self.digest.strip_prefix("sha256:")?;
        if !is_hex64(h) {
            return None;
        }
        let mut d = [0u8; 32];
        hex::decode_to_slice(h, &mut d).ok()?;
        Some(d)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Platform {
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub arch: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub libc: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Language {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub framework: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Runtime {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    #[serde(default)]
    pub included: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub digest: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Process {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub workdir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<Port>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<Health>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Port {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub env: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub default: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub protocol: String,
}

impl Port {
    pub fn http(default: i64) -> Self {
        Port { env: "PORT".into(), default, protocol: "http".into() }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Health {
    #[serde(default)]
    pub path: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Static {
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub at: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub immutable: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Env {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub optional: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub defaults: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Resources {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub memory: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Build {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub revision: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub builder: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub time: String,
}

/// letters, digits, `.`, `-`, `_`; 1 to 128 of them (§5.1).
pub fn valid_name(s: &str) -> bool {
    (1..=128).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
}

impl Manifest {
    /// Says whether a manifest follows §5.
    pub fn check(&self) -> Result<()> {
        if self.odi.is_empty() {
            bail!("manifest: odi (the version) is required");
        }
        if !valid_name(&self.name) {
            bail!("manifest: name: letters, digits, . - _ (at most 128)");
        }
        if self.version.len() > 128 {
            bail!("manifest: version: at most 128 characters");
        }
        if self.processes.is_empty() && self.r#static.is_empty() && self.provides.is_empty() && self.parts.is_empty() {
            bail!("manifest: a process to run, static files to serve, parts, or something it provides");
        }
        let mut names = std::collections::HashSet::new();
        for p in &self.parts {
            if !valid_name(&p.name) || !names.insert(p.name.as_str()) {
                bail!("manifest: part {:?}: a unique name (letters, digits, . - _)", p.name);
            }
            if p.digest_bytes().is_none() {
                bail!("manifest: part {}: digest {:?} (sha256: and 64 hex digits)", p.name, p.digest);
            }
            if !p.role.is_empty() && !ROLES.contains(&p.role.as_str()) {
                bail!("manifest: part {}: as {:?} (main, runtime, overlay, or none)", p.name, p.role);
            }
        }
        for p in self.provides.iter().chain(&self.needs) {
            if p.plug.is_empty() || p.name.is_empty() {
                bail!("manifest: a plug needs plug and name");
            }
        }
        let mut seen = std::collections::HashSet::new();
        for p in &self.processes {
            if p.name.is_empty() || !seen.insert(p.name.as_str()) || p.command.is_empty() {
                bail!("manifest: process {:?}: a unique name and a command", p.name);
            }
            if !p.workdir.is_empty() && check_path(&p.workdir).is_err() {
                bail!("manifest: process {}: bad workdir {:?}", p.name, p.workdir);
            }
        }
        for s in &self.r#static {
            if check_path(&s.from).is_err() || !s.at.starts_with('/') {
                bail!("manifest: static {:?} at {:?}", s.from, s.at);
            }
        }
        for w in &self.writable {
            if check_path(w).is_err() || !w.starts_with("app/") {
                bail!("manifest: writable {:?}: a folder under app/", w);
            }
        }
        if let Some(env) = &self.env {
            for k in env.defaults.keys() {
                if k.is_empty() || k.contains('=') || k.contains('\0') {
                    bail!("manifest: env default {:?}", k);
                }
            }
        }
        Ok(())
    }
}

/// Relative, `/`-separated, no `.` or `..` part, no leading `/` (§3.4).
pub fn check_path(p: &str) -> Result<()> {
    if p.is_empty() || p.starts_with('/') || p.contains('\\') || p.contains('\0') {
        bail!("bad path {:?}", p);
    }
    if p.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
        bail!("bad path {:?}", p);
    }
    Ok(())
}

/// What may be at the top of an ODI file (§4).
pub const TOP_LEVEL: [&str; 4] = ["app", "static", "runtime", "meta"];

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Checks an entry's path, type, mode and (for a symlink) target.
pub fn check_entry(e: &Entry) -> Result<()> {
    check_path(&e.path)?;
    let top = e.path.split('/').next().unwrap_or("");
    if !TOP_LEVEL.contains(&top) {
        bail!("{}: only app/, static/, runtime/ and meta/ at the top", e.path);
    }
    if e.mode & !0o777 != 0 {
        bail!("{}: mode {:o} (permission bits only)", e.path, e.mode);
    }
    match e.kind {
        Kind::Dir => {}
        Kind::File => {
            let mut n: u64 = 0;
            for c in &e.chunks {
                if !is_hex64(&c.hash) || c.size == 0 || c.size > MAX_CHUNK {
                    bail!("{}: a bad chunk", e.path);
                }
                n += c.size; // (at most 2M entries of 4 MiB chunks: can't overflow)
            }
            if n != e.size {
                bail!("{}: chunks hold {} bytes, not {}", e.path, n, e.size);
            }
        }
        Kind::Symlink => {
            if e.target.is_empty() || e.target.starts_with('/') {
                bail!("{}: a symlink's target must be relative", e.path);
            }
            // (Resolved from the link's folder, it must stay inside the file.)
            let resolved = clean(&format!("{}/{}", parent(&e.path), e.target));
            if resolved == ".." || resolved.starts_with("../") || resolved == "." {
                bail!("{}: its target {:?} points outside", e.path, e.target);
            }
        }
    }
    Ok(())
}

/// A path's folder, as Go's path.Dir: "." for a top-level name.
pub(crate) fn parent(p: &str) -> &str {
    match p.rfind('/') {
        Some(i) => &p[..i],
        None => ".",
    }
}

/// A relative path with `.`, `..` and repeated `/` worked out, as path.Clean.
pub(crate) fn clean(p: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for part in p.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if matches!(out.last(), Some(&last) if last != "..") {
                    out.pop();
                } else {
                    out.push("..");
                }
            }
            _ => out.push(part),
        }
    }
    if out.is_empty() {
        ".".into()
    } else {
        out.join("/")
    }
}

/// What one entry can't show alone (§3.4): nothing is stored under a file or
/// a link, and a link, followed through any links on its way, stays inside
/// the file. Without this, links that each look fine can leave the folder
/// they're unpacked into.
pub(crate) fn check_tree(m: &Manifest, entries: &[Entry]) -> Result<()> {
    let mut kinds: HashMap<&str, Kind> = HashMap::with_capacity(entries.len());
    let mut links: HashMap<&str, &str> = HashMap::new();
    for e in entries {
        kinds.insert(&e.path, e.kind);
        if e.kind == Kind::Symlink {
            links.insert(&e.path, &e.target);
        }
    }
    let under = |p: &str| -> Result<()> {
        let mut at = parent(p);
        while at != "." {
            if let Some(&k) = kinds.get(at) {
                if k != Kind::Dir {
                    bail!("{}: under the {} {}", p, k, at);
                }
            }
            at = parent(at);
        }
        Ok(())
    };
    for e in entries {
        under(&e.path)?;
    }
    for w in &m.writable {
        under(w)?;
    }
    for (p, target) in &links {
        follow_link(p, target, &links)?;
    }
    Ok(())
}

/// Walks a link's target as a filesystem would.
fn follow_link(p: &str, target: &str, links: &HashMap<&str, &str>) -> Result<()> {
    let outside = || Error(format!("{}: its target {:?} points outside", p, target));
    let mut at: Vec<&str> = match parent(p) {
        "." => Vec::new(),
        d => d.split('/').collect(),
    };
    // What's left to walk, the next part last.
    let mut rest: Vec<&str> = target.split('/').rev().collect();
    let mut hops = 0;
    while let Some(part) = rest.pop() {
        match part {
            "" | "." => {}
            ".." => {
                if at.pop().is_none() {
                    return Err(outside());
                }
            }
            _ => {
                at.push(part);
                if let Some(t) = links.get(at.join("/").as_str()) {
                    hops += 1;
                    if hops > MAX_LINKS {
                        bail!("{}: too many links on the way to {:?}", p, target);
                    }
                    at.pop();
                    rest.extend(t.split('/').rev());
                }
            }
        }
    }
    if at.is_empty() {
        return Err(outside());
    }
    Ok(())
}

#[cfg(all(test, feature = "zstd"))]
mod tests;
