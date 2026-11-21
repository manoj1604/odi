//! A registry, ODI Hub by default (docs/registry.md): odi login, push, pull, tags.
//!
//! A file goes up as exact byte pieces: each stored chunk, and the bytes
//! around them (the header, then the manifest, index and footer). Only the
//! pieces the registry lacks are sent, and a pull puts the same bytes back in
//! order. What's identical between versions is sent and stored once.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use odi_format::{Manifest, ReadAt, Reader};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};

use crate::flags::{self, flag, Kind::*};
use crate::{host_arch, host_os, human};

const DEFAULT_REGISTRY: &str = "https://hub.odisssi.com";
/// Where the hub's API keys are made.
const KEYS_URL: &str = "https://dev.odisssi.com/dashboard#keys";

/// The largest README a repository's page shows.
const MAX_README: u64 = 256 << 10;

/// Pieces moved at once.
const TRANSFERS: usize = 8;

/// The most one answer from a registry may hold.
const MAX_RESPONSE: u64 = 1 << 30;

/// NAMESPACE/NAME[:TAG|@sha256:DIGEST], with a registry before it if its
/// first part looks like a host.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Ref {
    pub registry: String,
    pub ns: String,
    pub name: String,
    pub tag: String,
    pub digest: String,
}

impl Ref {
    pub fn repo(&self) -> String {
        format!("{}/{}", self.ns, self.name)
    }
}

pub fn parse_ref(s: &str) -> Result<Ref> {
    let bad = || anyhow!("{s:?}: a name is NAMESPACE/NAME, with :TAG or @sha256:DIGEST if wanted");
    let mut r = Ref::default();
    let mut rest = s;
    if let Some(i) = rest.rfind('@') {
        let d = rest[i + 1..].strip_prefix("sha256:").ok_or_else(bad)?;
        if d.len() != 64 || !d.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(bad());
        }
        r.digest = d.to_ascii_lowercase();
        rest = &rest[..i];
    }
    let mut parts: Vec<&str> = rest.split('/').collect();
    if parts.len() == 3 && parts[0].contains(['.', ':']) {
        r.registry = format!("https://{}", parts[0]);
        parts.remove(0);
    }
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        return Err(bad());
    }
    r.ns = parts[0].into();
    let (name, tag) = parts[1].split_once(':').unwrap_or((parts[1], ""));
    if name.is_empty() || tag.contains(':') {
        return Err(bad());
    }
    r.name = name.into();
    r.tag = tag.into();
    if r.tag.is_empty() && r.digest.is_empty() {
        r.tag = "latest".into();
    }
    Ok(r)
}

/// null as the type's empty value (a registry may send either).
pub fn nullable<'de, D: Deserializer<'de>, T: Default + Deserialize<'de>>(d: D) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

// --- signing in ------------------------------------------------------------------

/// odi's own settings: ~/.odi/config.json ($ODI_HOME).
#[derive(Serialize, Deserialize, Default)]
struct Config {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    registry: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    token: String,
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default()
}

pub fn odi_home() -> PathBuf {
    std::env::var_os("ODI_HOME").map(PathBuf::from).unwrap_or_else(|| home_dir().join(".odi"))
}

fn load_config() -> Config {
    fs::read(odi_home().join("config.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_config(c: &Config) -> Result<()> {
    let home = odi_home();
    fs::create_dir_all(&home)?;
    crate::set_mode(&home, 0o700)?;
    let p = home.join("config.json");
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(&p)?.write_all(&serde_json::to_vec_pretty(c)?)?;
    Ok(())
}

/// The account ody is signed in to, if it is: one login.
fn ody_sign_in() -> (String, String) {
    let home = std::env::var_os("ODY_HOME").map(PathBuf::from).unwrap_or_else(|| home_dir().join(".ody"));
    #[derive(Deserialize, Default)]
    struct C {
        #[serde(default)]
        token: String,
        #[serde(default)]
        aska_org: String,
    }
    let c: C =
        fs::read(home.join("config.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    (c.token, c.aska_org)
}

/// A registry's answer that wasn't 2xx.
#[derive(Debug)]
pub struct StatusError {
    pub status: u16,
    pub msg: String,
}

impl std::fmt::Display for StatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for StatusError {}

/// Whether an error is the registry answering with this status.
pub fn is_status(e: &anyhow::Error, status: u16) -> bool {
    e.downcast_ref::<StatusError>().is_some_and(|s| s.status == status)
}

/// Talks to one registry.
pub struct Client {
    base: String,
    token: String,
    org: String,
    agent: ureq::Agent,
}

pub fn new_client(r: &Ref) -> Client {
    let c = load_config();
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let base = [r.registry.clone(), env("ODI_REGISTRY"), c.registry.clone(), DEFAULT_REGISTRY.into()]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or_default();
    let (ody_token, ody_org) = ody_sign_in();
    let mut token = c.token;
    if !env("ODI_TOKEN").is_empty() {
        token = env("ODI_TOKEN");
    }
    if token.is_empty() {
        token = ody_token.clone();
    }
    let mut org = env("ODY_ORG");
    if org.is_empty() && token == ody_token {
        org = ody_org;
    }
    // (A registry that doesn't answer is said so in seconds, not minutes;
    // a big transfer still has five.)
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .user_agent(&format!("odi/{}", crate::VERSION))
        .build();
    Client { base: base.trim_end_matches('/').to_string(), token, org, agent }
}

impl Client {
    pub fn signed_in(&self) -> bool {
        !self.token.is_empty()
    }

    pub fn call(&self, method: &str, path: &str, body: Option<&[u8]>, content_type: Option<&str>) -> Result<Vec<u8>> {
        self.call_with(method, path, body, content_type, &[])
    }

    /// As call, with more headers (If-Match).
    pub fn call_with(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        content_type: Option<&str>,
        headers: &[(&str, &str)],
    ) -> Result<Vec<u8>> {
        let mut last = anyhow!("no answer");
        for attempt in 0..4u64 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(attempt * 500));
            }
            let mut req = self.agent.request(method, &format!("{}{}", self.base, path));
            if !self.token.is_empty() {
                req = req.set("Authorization", &format!("Bearer {}", self.token));
                if !self.org.is_empty() {
                    req = req.set("X-Ody-Org", &self.org);
                }
            }
            if let Some(ct) = content_type {
                req = req.set("Content-Type", ct);
            }
            for (k, v) in headers {
                req = req.set(k, v);
            }
            let res = match body {
                Some(b) => req.send_bytes(b),
                None => req.call(),
            };
            let (status, resp) = match res {
                Ok(resp) => (resp.status(), resp),
                Err(ureq::Error::Status(code, resp)) => (code, resp),
                Err(e) => {
                    last = anyhow!("can't reach {}: {e}", self.base);
                    continue;
                }
            };
            let status_text = format!("{} {}", status, resp.status_text());
            let mut data = Vec::new();
            if let Err(e) = resp.into_reader().take(MAX_RESPONSE).read_to_end(&mut data) {
                last = e.into();
                continue;
            }
            if status / 100 == 2 {
                return Ok(data);
            }
            #[derive(Deserialize, Default)]
            struct E {
                #[serde(default)]
                error: String,
            }
            let mut msg = serde_json::from_slice::<E>(&data).unwrap_or_default().error;
            if msg.is_empty() {
                msg = status_text;
            }
            last = StatusError { status, msg }.into();
            if status < 500 && status != 429 {
                return Err(last);
            }
        }
        Err(last)
    }

    pub fn json<T: DeserializeOwned>(&self, method: &str, path: &str, body: Option<&Value>) -> Result<T> {
        let data = self.send_json(method, path, body)?;
        Ok(serde_json::from_slice(&data)?)
    }

    pub fn send_json(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Vec<u8>> {
        match body {
            Some(v) => self.call(method, path, Some(&serde_json::to_vec(v)?), Some("application/json")),
            None => self.call(method, path, None, None),
        }
    }
}

pub fn cmd_login(args: &[String]) -> Result<()> {
    const U: &str = "odi login [--token TOKEN] [--registry URL]";
    let specs = [
        flag("token", Value, "an account token, or an organization's API key"),
        flag("registry", Value, "another registry's address"),
    ];
    let (f, _) = flags::parse(&specs, args, U)?;
    let mut c = load_config();
    if let Some(r) = f.str("registry").filter(|r| !r.is_empty()) {
        c.registry = r.trim_end_matches('/').into();
    }
    let mut token = f.str("token").unwrap_or("").to_string();
    if token.is_empty() {
        token = std::env::var("ODI_TOKEN").unwrap_or_default();
    }
    if token.is_empty() && !ody_sign_in().0.is_empty() {
        save_config(&c)?;
        println!("Using ody's sign-in.");
        return Ok(());
    }
    if token.is_empty() {
        let hub = c.registry.is_empty() || c.registry.trim_end_matches('/') == DEFAULT_REGISTRY;
        if hub && std::io::IsTerminal::is_terminal(&io::stdin()) {
            eprintln!("Paste an API key from {KEYS_URL} (no account yet? sign up there first).");
        }
        token = ask_token()?;
    }
    if token.is_empty() {
        bail!("no token: odi login --token TOKEN, or set ODI_TOKEN (a token or API key from your registry)");
    }
    c.token = token;
    save_config(&c)?;
    println!("Signed in.");
    Ok(())
}

/// A token typed at the terminal, not shown; nothing when not at one.
fn ask_token() -> Result<String> {
    use std::io::{BufRead, IsTerminal};
    if !io::stdin().is_terminal() {
        return Ok(String::new());
    }
    eprint!("Token: ");
    io::stderr().flush()?;
    #[cfg(unix)]
    let saved = {
        // SAFETY: termios is plain data; fd 0 is a terminal (checked above).
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(0, &mut t) == 0 {
                let old = t;
                t.c_lflag &= !libc::ECHO;
                libc::tcsetattr(0, libc::TCSANOW, &t);
                Some(old)
            } else {
                None
            }
        }
    };
    let mut line = String::new();
    let r = io::stdin().lock().read_line(&mut line);
    #[cfg(unix)]
    if let Some(old) = saved {
        // SAFETY: as above, restoring what was there.
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &old) };
    }
    eprintln!();
    r?;
    Ok(line.trim().to_string())
}

pub fn cmd_logout() -> Result<()> {
    let mut c = load_config();
    c.token.clear();
    save_config(&c)?;
    println!("Signed out of odi.");
    Ok(())
}

// --- the pieces of a file ----------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub hash: String,
    pub length: u64,
    #[serde(skip)]
    pub off: u64,
}

/// Cuts a file into its stored chunks and the bytes around them, in order,
/// covering it exactly.
pub fn segments(rd: &Reader) -> Result<Vec<Segment>> {
    let mut seen: BTreeMap<u64, u64> = BTreeMap::new();
    for e in &rd.entries {
        for c in &e.chunks {
            if let Some(&n) = seen.get(&c.offset) {
                if n != c.length {
                    bail!("two chunks at one place in the file");
                }
            }
            seen.insert(c.offset, c.length);
        }
    }
    let mut out = Vec::new();
    let mut add = |off: u64, n: u64| -> Result<()> {
        let mut b = vec![0u8; n as usize];
        rd.read_at(&mut b, off)?;
        out.push(Segment { hash: hex::encode(Sha256::digest(&b)), length: n, off });
        Ok(())
    };
    let mut at = 0u64;
    for (&o, &n) in &seen {
        if o < at {
            bail!("chunks overlap in the file");
        }
        if o > at {
            add(at, o - at)?;
        }
        add(o, n)?;
        at = o + n;
    }
    if at < rd.size() {
        add(at, rd.size() - at)?;
    }
    Ok(out)
}

/// Runs f on 0..n, a few at a time; the first error stops it.
pub fn each(n: usize, f: impl Fn(usize) -> Result<()> + Sync) -> Result<()> {
    let next = AtomicUsize::new(0);
    let first: Mutex<Option<anyhow::Error>> = Mutex::new(None);
    std::thread::scope(|s| {
        for _ in 0..TRANSFERS {
            s.spawn(|| loop {
                if first.lock().expect("lock").is_some() {
                    return;
                }
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= n {
                    return;
                }
                if let Err(e) = f(i) {
                    first.lock().expect("lock").get_or_insert(e);
                }
            });
        }
    });
    match first.into_inner().expect("lock") {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

// --- push and pull -----------------------------------------------------------------

/// The one .odi file here, when a command isn't given one.
pub fn only_odi_file() -> Result<String> {
    let found: Vec<String> = fs::read_dir(".")?
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".odi"))
        .collect();
    if found.len() != 1 {
        bail!("found {} .odi files here: name the one to push", found.len());
    }
    Ok(found.into_iter().next().expect("one"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagPlatform {
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub digest: String,
}

pub fn platform_of(m: &Manifest) -> String {
    match &m.platform {
        None => "any/any".into(),
        Some(p) => format!("{}/{}", p.os, p.arch),
    }
}

/// Sends one file's missing pieces and finishes it (no tag yet).
pub fn push_file(c: &Client, base: &str, path: &str) -> Result<TagPlatform> {
    let rd = Reader::open(path)?;
    let segs = segments(&rd)?;
    let digest = hex::encode(rd.digest);
    let manifest: Value = serde_json::from_slice(&rd.manifest_json).unwrap_or(Value::Null);
    let readme = rd
        .entry("meta/README.md")
        .filter(|e| e.size <= MAX_README)
        .and_then(|e| rd.read_file(&e.path).ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    #[derive(Deserialize)]
    struct Started {
        #[serde(default)]
        ready: bool,
        #[serde(default, deserialize_with = "nullable")]
        missing: Vec<String>,
    }
    let body = json!({"digest": format!("sha256:{digest}"), "size": rd.size(), "segments": segs, "manifest": manifest, "readme": readme});
    let started: Started = c.json("POST", &format!("{base}/digests"), Some(&body))?;
    let by_hash: HashMap<&str, &Segment> = segs.iter().map(|s| (s.hash.as_str(), s)).collect();
    let sent = AtomicU64::new(0);
    each(started.missing.len(), |i| {
        let Some(s) = by_hash.get(started.missing[i].as_str()) else {
            bail!("the registry asked for a piece this file doesn't have");
        };
        let mut b = vec![0u8; s.length as usize];
        rd.read_at(&mut b, s.off)?;
        c.call("PUT", &format!("{base}/segments/{}", s.hash), Some(&b), Some("application/octet-stream"))?;
        sent.fetch_add(s.length, Ordering::Relaxed);
        Ok(())
    })?;
    if !started.ready {
        c.send_json("POST", &format!("{base}/digests/{digest}/commit"), None)?;
    }
    let platform = platform_of(&rd.manifest);
    println!(
        "{platform:<14} sha256:{digest}  {} in the file, {} sent",
        human(rd.size()),
        human(sent.load(Ordering::Relaxed))
    );
    Ok(TagPlatform { platform, digest })
}

/// The digest a name means here: its own, or the tag's build for this
/// platform (or one for any platform).
fn resolve(c: &Client, r: &Ref, platform: &str) -> Result<String> {
    if !r.digest.is_empty() {
        return Ok(r.digest.clone());
    }
    #[derive(Deserialize)]
    struct Tag {
        #[serde(default, deserialize_with = "nullable")]
        platforms: Vec<TagPlatform>,
        /// (A registry that can sends its builds' lists of pieces too: the
        /// request for one is then saved.)
        #[serde(default)]
        builds: HashMap<String, Build>,
    }
    let get = |tag: &str| -> Result<Tag> {
        c.json("GET", &format!("/v1/repos/{}/tags/{}?builds=1", r.repo(), path_escape(tag)), None)
    };
    let (t, tag) = match get(&r.tag) {
        Ok(t) => (t, r.tag.clone()),
        // (1.4, ^1.4, ~1.4.2: the newest version in the range.)
        Err(e) if is_status(&e, 404) && crate::versions::Range::parse(&r.tag).is_some() => {
            let tag = crate::publish::pick_version(c, r)?;
            eprintln!("odi: {}:{} is {tag}", r.repo(), r.tag);
            (get(&tag)?, tag)
        }
        Err(e) if is_status(&e, 404) => {
            return Err(e.context(format!("{}:{} (odi versions {} lists them)", r.repo(), r.tag, r.repo())))
        }
        Err(e) => return Err(e),
    };
    for (d, b) in t.builds {
        remember_build(&d, b);
    }
    if let Some(p) = t
        .platforms
        .iter()
        .find(|p| p.platform == platform)
        .or_else(|| t.platforms.iter().find(|p| p.platform == "any/any"))
    {
        return Ok(p.digest.clone());
    }
    let has: Vec<&str> = t.platforms.iter().map(|p| p.platform.as_str()).collect();
    bail!("{}:{tag} has no build for {platform} (it has {})", r.repo(), has.join(", "))
}

/// What Go's url.PathEscape leaves as it is: unreserved characters and the
/// sub-delimiters a path segment may hold.
const PATH_SEGMENT: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'$')
    .remove(b'&')
    .remove(b'+')
    .remove(b',')
    .remove(b';')
    .remove(b'=')
    .remove(b':')
    .remove(b'@');

pub fn path_escape(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, PATH_SEGMENT).to_string()
}

#[derive(Default, Serialize, Deserialize)]
pub struct Build {
    #[serde(default)]
    pub size: u64,
    #[serde(default, deserialize_with = "nullable")]
    pub segments: Vec<Segment>,
    /// A set's parts' builds, by digest, when the registry sends them.
    #[serde(default, skip_serializing)]
    pub parts: HashMap<String, Build>,
}

impl Client {
    fn build(&self, base: &str, digest: &str) -> Result<Build> {
        let mut b: Build = self.json("GET", &format!("{base}/digests/{digest}?parts=1"), None)?;
        for (d, pb) in std::mem::take(&mut b.parts) {
            remember_build(&d, pb);
        }
        checked_build(b)
    }

    /// Many pieces in one request (`POST .../segments/batch`, the hashes in
    /// order; the answer is the pieces back to back), each checked.
    fn pieces(&self, base: &str, segs: &[&Segment]) -> Result<Vec<Vec<u8>>> {
        let hashes: Vec<&str> = segs.iter().map(|s| s.hash.as_str()).collect();
        let data = self.send_json("POST", &format!("{base}/segments/batch"), Some(&json!({ "hashes": hashes })))?;
        if data.len() as u64 != segs.iter().map(|s| s.length).sum::<u64>() {
            bail!("the registry's batch of pieces came back the wrong size");
        }
        let mut out = Vec::with_capacity(segs.len());
        let mut at = 0usize;
        for s in segs {
            let d = &data[at..at + s.length as usize];
            if hex::encode(Sha256::digest(d)) != s.hash {
                bail!("piece {} came back wrong", &s.hash[..12.min(s.hash.len())]);
            }
            out.push(d.to_vec());
            at += s.length as usize;
        }
        Ok(out)
    }

    /// One piece, checked against its hash.
    fn piece(&self, base: &str, s: &Segment) -> Result<Vec<u8>> {
        let data = self.call("GET", &format!("{base}/segments/{}", s.hash), None, None)?;
        if data.len() as u64 != s.length || hex::encode(Sha256::digest(&data)) != s.hash {
            bail!("piece {} came back wrong", &s.hash[..12.min(s.hash.len())]);
        }
        Ok(data)
    }
}

/// Lays out a build's list of pieces (each one's offset), checking it adds up
/// and that every name is a SHA-256 (a piece's name is a file's name in the
/// piece cache).
pub fn checked_build(mut b: Build) -> Result<Build> {
    let mut at = 0u64;
    for s in &mut b.segments {
        if s.length == 0 || s.length > MAX_RESPONSE || !is_sha256_hex(&s.hash) {
            bail!("the registry's list of pieces doesn't add up");
        }
        s.off = at;
        at = at.checked_add(s.length).ok_or_else(|| anyhow!("the registry's list of pieces doesn't add up"))?;
    }
    if at != b.size {
        bail!("the registry's list of pieces doesn't add up");
    }
    Ok(b)
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// This machine's pieces from registries, kept on disk by hash: shared by
/// every file and every run (an app's second cold start, or another app on
/// the same runtime, reads them here). `odi fill` puts them here ahead of a
/// first run. ODI_PIECE_CACHE=0: none.
pub struct PieceCache {
    dir: PathBuf,
}

impl PieceCache {
    pub fn open() -> Option<PieceCache> {
        if std::env::var("ODI_PIECE_CACHE").is_ok_and(|v| v == "0") {
            return None;
        }
        crate::run::cache_base().ok().map(|b| PieceCache::at(b.join("pieces")))
    }

    pub fn at(dir: PathBuf) -> PieceCache {
        PieceCache { dir }
    }

    fn path(&self, hash: &str) -> PathBuf {
        self.dir.join(&hash[..2]).join(hash)
    }

    /// A piece, if it's here and whole (one that isn't is thrown away).
    pub fn get(&self, s: &Segment) -> Option<Vec<u8>> {
        let p = self.path(&s.hash);
        let d = fs::read(&p).ok()?;
        if d.len() as u64 != s.length || hex::encode(Sha256::digest(&d)) != s.hash {
            let _ = fs::remove_file(&p);
            return None;
        }
        Some(d)
    }

    /// Keeps a piece (already checked), if it can.
    pub fn put(&self, s: &Segment, d: &[u8]) {
        let p = self.path(&s.hash);
        if p.exists() {
            return;
        }
        let tmp = p.with_extension(format!("tmp-{}-{:?}", std::process::id(), std::thread::current().id()));
        let ok = fs::create_dir_all(p.parent().expect("in a folder")).is_ok() && fs::write(&tmp, d).is_ok();
        if !ok || fs::rename(&tmp, &p).is_err() {
            let _ = fs::remove_file(&tmp);
        }
    }
}

/// Builds' lists of pieces a registry sent along with something else (a
/// tag's, a set's parts'), until they're asked for.
static SENT: Mutex<Option<HashMap<String, Build>>> = Mutex::new(None);

fn remember_build(digest: &str, b: Build) {
    let d = digest.strip_prefix("sha256:").unwrap_or(digest);
    if is_sha256_hex(d) {
        SENT.lock().expect("lock").get_or_insert_with(HashMap::new).insert(d.to_string(), b);
    }
}

/// A build's list of pieces as kept here, by registry, repository and digest.
/// (Not by digest alone: a signed copy has the digest of the unsigned one and
/// other bytes at its end.) What a registry just sent replaces what's kept.
fn build_cached(c: &Client, base: &str, digest: &str) -> Result<Build> {
    let path = build_path(c, base, digest);
    let sent = SENT.lock().expect("lock").as_mut().and_then(|m| m.remove(digest));
    if let Some(b) = sent.and_then(|b| checked_build(b).ok()) {
        save_build(path.as_deref(), &b);
        return Ok(b);
    }
    if let Some(b) = path.as_ref().and_then(|p| fs::read(p).ok()).and_then(|j| serde_json::from_slice::<Build>(&j).ok())
    {
        if let Ok(b) = checked_build(b) {
            return Ok(b);
        }
    }
    let b = c.build(base, digest)?;
    save_build(path.as_deref(), &b);
    Ok(b)
}

fn build_path(c: &Client, base: &str, digest: &str) -> Option<PathBuf> {
    if !is_sha256_hex(digest) {
        return None;
    }
    let at = hex::encode(Sha256::digest(format!("{}\n{base}", c.base)));
    Some(crate::run::cache_base().ok()?.join("builds").join(format!("{digest}-{}.json", &at[..16])))
}

fn save_build(path: Option<&Path>, b: &Build) {
    let Some(p) = path else { return };
    let _ = fs::create_dir_all(p.parent().expect("in a folder"));
    let tmp = p.with_extension(format!("tmp-{}", std::process::id()));
    if serde_json::to_vec(b).ok().is_some_and(|j| fs::write(&tmp, j).is_ok()) {
        let _ = fs::rename(&tmp, p);
    }
}

/// Reads a file in a registry at any offset, fetching the pieces it needs,
/// each once and checked. What a reader will want can be fetched ahead in
/// batches ([`Prefetch`]): many pieces in one request, so a program's start
/// costs a round trip, not one per piece.
pub struct PieceReader {
    c: Client,
    base: String,
    b: Build,
    pub size: u64,
    pub fetched: AtomicU64,
    /// Requests made for pieces (one per piece, or one per batch).
    pub requests: AtomicU64,
    /// The pieces fetched or being fetched, and their bytes.
    pieces: Mutex<(HashMap<usize, Arc<Piece>>, u64)>,
    /// The registry has no batch request: fetch pieces one by one, in parallel.
    no_batch: AtomicBool,
    /// $ODI_TRACE: a line per piece fetched.
    trace: Option<Mutex<fs::File>>,
    /// A piece's offset -> the file it's part of.
    labels: OnceLock<HashMap<u64, String>>,
    cache: Option<PieceCache>,
    /// Bytes read from the piece cache, not the registry.
    pub cached: AtomicU64,
}

/// A piece, fetched once however many readers want it at the same time.
/// While a batch has claimed it, readers wait for the batch.
#[derive(Default)]
struct Piece {
    data: OnceLock<Result<Vec<u8>, String>>,
    claimed: Mutex<bool>,
    ready: Condvar,
}

const PIECE_CACHE_BYTES: u64 = 512 << 20;

/// The most one batch request asks for.
const BATCH_BYTES: u64 = 8 << 20;

/// Fetches ahead what a reader will read: (offset, length) ranges of the file.
pub trait Prefetch: Send + Sync {
    fn prefetch(&self, ranges: &[(u64, u64)]);
    /// What's been fetched so far.
    fn stats(&self) -> Stats;
}

/// Bytes fetched, requests made, and the size of what's read from.
#[derive(Default, Clone, Copy)]
pub struct Stats {
    pub fetched: u64,
    pub requests: u64,
    pub size: u64,
    /// Read from this machine's piece cache instead.
    pub cached: u64,
}

impl PieceReader {
    fn slot(&self, i: usize) -> Arc<Piece> {
        let mut g = self.pieces.lock().expect("lock");
        if !g.0.contains_key(&i) {
            if g.1 > PIECE_CACHE_BYTES {
                *g = (HashMap::new(), 0);
            }
            g.0.insert(i, Arc::default());
            g.1 += self.b.segments[i].length;
        }
        g.0[&i].clone()
    }

    fn got(&self, s: &Segment) {
        self.fetched.fetch_add(s.length, Ordering::Relaxed);
        if let Some(t) = &self.trace {
            let _ = writeln!(t.lock().expect("lock"), "{} {}", s.length, self.label(s));
        }
    }

    fn get(&self, i: usize) -> io::Result<Arc<Piece>> {
        let p = self.slot(i);
        {
            let mut claimed = p.claimed.lock().expect("lock");
            while *claimed && p.data.get().is_none() {
                claimed = p.ready.wait(claimed).expect("lock");
            }
        }
        p.data.get_or_init(|| {
            let s = &self.b.segments[i];
            if let Some(d) = self.local(s) {
                return Ok(d);
            }
            self.requests.fetch_add(1, Ordering::Relaxed);
            let data = self.c.piece(&self.base, s).map_err(|e| format!("{e:#}"))?;
            self.got(s);
            if let Some(c) = &self.cache {
                c.put(s, &data);
            }
            Ok(data)
        });
        if let Some(Err(e)) = p.data.get() {
            return Err(io::Error::other(e.clone()));
        }
        Ok(p)
    }

    /// Hands a claimed piece what was fetched for it, and wakes its readers.
    fn fill(&self, i: usize, p: &Piece, r: Result<Vec<u8>, String>) {
        if let Ok(d) = &r {
            self.got(&self.b.segments[i]);
            if let Some(c) = &self.cache {
                c.put(&self.b.segments[i], d);
            }
        }
        let _ = p.data.set(r);
        *p.claimed.lock().expect("lock") = false;
        p.ready.notify_all();
    }

    /// Fetches claimed pieces: in one request if the registry can, else one
    /// by one, a few at a time.
    fn fetch_batch(&self, batch: &[(usize, Arc<Piece>)]) {
        if !self.no_batch.load(Ordering::Relaxed) {
            let segs: Vec<&Segment> = batch.iter().map(|(i, _)| &self.b.segments[*i]).collect();
            self.requests.fetch_add(1, Ordering::Relaxed);
            match self.c.pieces(&self.base, &segs) {
                Ok(datas) => {
                    for ((i, p), d) in batch.iter().zip(datas) {
                        self.fill(*i, p, Ok(d));
                    }
                    return;
                }
                Err(e) => {
                    if e.downcast_ref::<StatusError>().is_some_and(|s| matches!(s.status, 404 | 405 | 501)) {
                        self.no_batch.store(true, Ordering::Relaxed);
                    }
                }
            }
        }
        let _ = each(batch.len(), |n| {
            let (i, p) = &batch[n];
            self.requests.fetch_add(1, Ordering::Relaxed);
            let r = self.c.piece(&self.base, &self.b.segments[*i]).map_err(|e| format!("{e:#}"));
            self.fill(*i, p, r);
            Ok(())
        });
    }

    /// A piece from this machine's piece cache.
    fn local(&self, s: &Segment) -> Option<Vec<u8>> {
        let d = self.cache.as_ref()?.get(s)?;
        self.cached.fetch_add(s.length, Ordering::Relaxed);
        Some(d)
    }

    fn label(&self, s: &Segment) -> String {
        if let Some(l) = self.labels.get().and_then(|m| m.get(&s.off)) {
            return l.clone();
        }
        if s.off == 0 { "(header)" } else { "(manifest, index, footer)" }.into()
    }
}

impl Prefetch for PieceReader {
    fn prefetch(&self, ranges: &[(u64, u64)]) {
        let segs = &self.b.segments;
        let mut want = std::collections::BTreeSet::new();
        for &(off, len) in ranges {
            let mut i = segs.partition_point(|s| s.off + s.length <= off);
            while len > 0 && i < segs.len() && segs[i].off < off.saturating_add(len) {
                want.insert(i);
                i += 1;
            }
        }
        // Claim what isn't here or on its way, so readers wait for the batch.
        let mut batches: Vec<Vec<(usize, Arc<Piece>)>> = vec![Vec::new()];
        let mut bytes = 0;
        for i in want {
            let p = self.slot(i);
            {
                let mut claimed = p.claimed.lock().expect("lock");
                if p.data.get().is_some() || *claimed {
                    continue;
                }
                *claimed = true;
            }
            if let Some(d) = self.local(&segs[i]) {
                let _ = p.data.set(Ok(d));
                *p.claimed.lock().expect("lock") = false;
                p.ready.notify_all();
                continue;
            }
            if bytes > 0 && bytes + segs[i].length > BATCH_BYTES {
                batches.push(Vec::new());
                bytes = 0;
            }
            bytes += segs[i].length;
            batches.last_mut().expect("one").push((i, p));
        }
        std::thread::scope(|s| {
            for b in batches.iter().filter(|b| !b.is_empty()) {
                s.spawn(|| self.fetch_batch(b));
            }
        });
    }

    fn stats(&self) -> Stats {
        Stats {
            fetched: self.fetched.load(Ordering::Relaxed),
            requests: self.requests.load(Ordering::Relaxed),
            size: self.size,
            cached: self.cached.load(Ordering::Relaxed),
        }
    }
}

impl ReadAt for PieceReader {
    fn read_exact_at(&self, buf: &mut [u8], off: u64) -> io::Result<()> {
        let segs = &self.b.segments;
        let mut i = segs.partition_point(|s| s.off + s.length <= off);
        let mut n = 0usize;
        while n < buf.len() {
            if i >= segs.len() {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            let p = self.get(i)?;
            let Some(Ok(d)) = p.data.get() else { return Err(io::Error::other("a piece went missing")) };
            let start = (off + n as u64 - segs[i].off) as usize;
            let k = (d.len() - start).min(buf.len() - n);
            buf[n..n + k].copy_from_slice(&d[start..start + k]);
            n += k;
            i += 1;
        }
        Ok(())
    }
}

/// Opens a file on disk, or, when there's none by that name, a name in a
/// registry (read as needed).
pub fn open_any(src: &str) -> Result<(Reader, Option<Arc<PieceReader>>)> {
    if Path::new(src).exists() {
        return Ok((Reader::open(src).with_context(|| src.to_string())?, None));
    }
    let r = parse_ref(src).map_err(|_| anyhow!("no file {src}, and it isn't a name in a registry"))?;
    let c = new_client(&r);
    let digest = resolve(&c, &r, &format!("{}/{}", host_os(), host_arch()))?;
    let base = format!("/v1/repos/{}", r.repo());
    let b = build_cached(&c, &base, &digest)?;
    let trace = match std::env::var_os("ODI_TRACE") {
        Some(t) => Some(Mutex::new(fs::OpenOptions::new().append(true).create(true).open(t)?)),
        None => None,
    };
    let size = b.size;
    let pr = Arc::new(PieceReader {
        c,
        base,
        b,
        size,
        fetched: AtomicU64::new(0),
        requests: AtomicU64::new(0),
        pieces: Mutex::new((HashMap::new(), 0)),
        no_batch: AtomicBool::new(false),
        trace,
        labels: OnceLock::new(),
        cache: PieceCache::open(),
        cached: AtomicU64::new(0),
    });
    // (One request: the header, the manifest, index and footer, and the small
    // pieces just before them, where builders put runner hints.)
    let mut first = vec![(0, 1), (size.saturating_sub(1), 1)];
    let segs = &pr.b.segments;
    for s in segs.iter().rev().skip(1).take(3).take_while(|s| s.length <= 256 << 10 && s.off > 0) {
        first.push((s.off, s.length));
    }
    pr.prefetch(&first);
    let rd = Reader::new(pr.clone(), size)?;
    let labels = rd.entries.iter().flat_map(|e| e.chunks.iter().map(move |c| (c.offset, e.path.clone()))).collect();
    let _ = pr.labels.set(labels);
    let got = hex::encode(rd.digest);
    if got != digest {
        bail!("the file's digest is {got}, not the {digest} asked for");
    }
    Ok((rd, Some(pr)))
}

#[cfg(unix)]
fn write_at(f: &fs::File, b: &[u8], off: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(f, b, off)
}

#[cfg(windows)]
fn write_at(f: &fs::File, mut b: &[u8], mut off: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !b.is_empty() {
        let n = f.seek_write(b, off)?;
        b = &b[n..];
        off += n as u64;
    }
    Ok(())
}

pub fn cmd_pull(args: &[String]) -> Result<()> {
    const U: &str = "odi pull NAMESPACE/NAME[:TAG|@sha256:DIGEST] [-o FILE] [--platform OS/ARCH]";
    let specs = [flag("o", Value, "where to write it"), flag("platform", Value, "the platform to get (OS/ARCH)")];
    let (f, pos) = flags::parse(&specs, args, U)?;
    if pos.len() != 1 {
        bail!("usage: {U}");
    }
    let r = parse_ref(&pos[0])?;
    let c = new_client(&r);
    let base = format!("/v1/repos/{}", r.repo());
    let plat = f.str("platform").map(String::from).unwrap_or_else(|| format!("{}/{}", host_os(), host_arch()));
    let digest = resolve(&c, &r, &plat)?;
    let b = c.build(&base, &digest)?;
    let path = PathBuf::from(f.str("o").map(String::from).unwrap_or_else(|| format!("{}.odi", r.name)));
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let tmp = tempfile::Builder::new().prefix(".odi-pull-").tempfile_in(&dir)?;
    each(b.segments.len(), |i| {
        let s = &b.segments[i];
        write_at(tmp.as_file(), &c.piece(&base, s)?, s.off)?;
        Ok(())
    })?;
    // (The digest is the proof: a registry only stores what it was sent.)
    let rd = Reader::new(tmp.as_file().try_clone()?, b.size).context("what came back isn't a good ODI file")?;
    let got = hex::encode(rd.digest);
    if got != digest {
        bail!("the file's digest is {got}, not the {digest} asked for");
    }
    rd.verify()?;
    crate::set_mode(tmp.path(), 0o644)?;
    tmp.persist(&path)?;
    println!("{}  sha256:{digest}  {}", path.display(), human(b.size));
    Ok(())
}

/// A tag, a build by digest, or with --repo all of a repository.
pub fn cmd_rm(args: &[String]) -> Result<()> {
    const U: &str = "odi rm NAMESPACE/NAME:TAG | NAMESPACE/NAME@sha256:DIGEST | NAMESPACE/NAME --repo";
    let specs = [flag("repo", Bool, "delete the repository, with every build")];
    let (f, pos) = flags::parse(&specs, args, U)?;
    if pos.len() != 1 {
        bail!("usage: {U}");
    }
    let r = parse_ref(&pos[0])?;
    let c = new_client(&r);
    let base = format!("/v1/repos/{}", r.repo());
    let last = &pos[0][pos[0].rfind('/').unwrap_or(0)..];
    let named = last.contains([':', '@']);
    let whole = f.bool("repo");
    if whole && named {
        bail!("--repo takes the repository's name alone");
    } else if whole {
        c.send_json("DELETE", &base, None)?;
        println!("deleted {} and its builds", r.repo());
    } else if !named {
        bail!("name a tag or a digest, or use --repo to delete all of {}", r.repo());
    } else if !r.digest.is_empty() {
        c.send_json("DELETE", &format!("{base}/digests/{}", r.digest), None)?;
        println!("deleted build sha256:{}", &r.digest[..12]);
    } else {
        c.send_json("DELETE", &format!("{base}/tags/{}", path_escape(&r.tag)), None)?;
        println!("deleted tag {} (its builds stay: odi rm NAME@sha256:DIGEST frees their space)", r.tag);
    }
    Ok(())
}

pub fn cmd_usage(args: &[String]) -> Result<()> {
    let r = match args {
        [one] => parse_ref(&format!("{one}/x"))?,
        _ => Ref::default(),
    };
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Limits {
        bytes: u64,
        file: u64,
        namespaces: u64,
        repos: u64,
    }
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Usage {
        bytes: u64,
        namespaces: u64,
        repos: u64,
        limits: Limits,
    }
    let u: Usage = new_client(&r).json("GET", "/v1/usage", None)?;
    println!(
        "storage     {} of {}\nnamespaces  {} of {}\nrepositories {} of {}\na build     up to {}",
        human(u.bytes),
        human(u.limits.bytes),
        u.namespaces,
        u.limits.namespaces,
        u.repos,
        u.limits.repos,
        human(u.limits.file)
    );
    Ok(())
}
