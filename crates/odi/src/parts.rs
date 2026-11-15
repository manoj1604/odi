//! Parts and sets (SPEC.md §5.9): an ODI file that lists other ODI
//! files by digest, put together into one tree when it runs.
//!
//! - A part with a role (`main`, `runtime`, `overlay`) is merged into the
//!   set's tree, in the order listed: a later file replaces an earlier one.
//!   One with none is under `parts/NAME/`. The set's own files go on top.
//! - A runtime pinned by digest (`runtime.digest`) is found the same way:
//!   a part of the set with that digest, or one in this machine's store.
//! - Parts come from the store (a folder of ODI files named by digest),
//!   else from the registry their `from` names, read as needed.
//!
//! What runs is one [`Reader`] over the whole tree: each part's bytes are
//! laid one after another in one address space, so mounting, unpacking,
//! start lists and route maps work on a set as on one file.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use odi_format::{digest_string, Entry, Kind, Manifest, Part, ReadAt, Reader, MAX_DEPTH};
use sha2::{Digest as _, Sha256};

use crate::hub::{PieceReader, Prefetch, Stats};

/// Where parts are kept on this machine: one file per digest.
pub fn store_dir() -> Result<PathBuf> {
    Ok(crate::run::cache_base()?.join("parts"))
}

pub fn store_path(d: &[u8; 32]) -> Result<PathBuf> {
    Ok(store_dir()?.join(format!("{}.odi", hex::encode(d))))
}

/// Copies an ODI file into the store (once: the same digest is the same
/// bytes that matter), every chunk checked. Returns its digest.
pub fn store_add(path: &Path) -> Result<[u8; 32]> {
    let rd = Reader::open(path).with_context(|| path.display().to_string())?;
    let to = store_path(&rd.digest)?;
    if !to.exists() {
        rd.verify().with_context(|| path.display().to_string())?;
        fs::create_dir_all(to.parent().expect("in a folder"))?;
        let mut tmp = to.clone().into_os_string();
        tmp.push(format!(".tmp-{}", std::process::id()));
        fs::copy(path, &tmp)?;
        fs::rename(&tmp, &to)?;
    }
    Ok(rd.digest)
}

/// A file read as needed: on disk, or from a registry.
pub struct Opened {
    pub rd: Reader,
    pub remote: Option<Arc<PieceReader>>,
}

/// Opens a part by digest: the store's copy, else from where it says.
pub fn open_part(p: &Part) -> Result<Opened> {
    let d = p.digest_bytes().ok_or_else(|| anyhow!("part {}: a bad digest", p.name))?;
    let local = store_path(&d)?;
    let (rd, remote) = if local.exists() {
        (Reader::open(&local).with_context(|| format!("part {} ({})", p.name, local.display()))?, None)
    } else if !p.from.is_empty() {
        // (A name in a registry, any tag dropped: the digest says which build.)
        let repo = repo_of(&p.from);
        crate::hub::open_any(&format!("{repo}@{}", p.digest)).with_context(|| format!("part {} from {repo}", p.name))?
    } else {
        bail!(
            "part {} ({}) isn't on this machine and doesn't say where to get it: odi part add FILE",
            p.name,
            p.digest
        );
    };
    if rd.digest != d {
        bail!("part {}: its digest is {}, not {}", p.name, digest_string(&rd.digest), p.digest);
    }
    Ok(Opened { rd, remote })
}

/// "acme/node:22" -> "acme/node" (a host with a port keeps its colon).
pub fn repo_of(r: &str) -> &str {
    let r = r.split('@').next().unwrap_or(r);
    match r.rfind(':') {
        Some(i) if !r[i..].contains('/') => &r[..i],
        _ => r,
    }
}

/// What runs: one tree, and how to fetch ahead what it will read.
pub struct Assembled {
    pub rd: Arc<Reader>,
    pub remote: Option<Arc<dyn Prefetch>>,
    /// Names this tree: the cache folder it unpacks into.
    pub key: String,
    /// The parts it was made of, in order (with the runtime when it came from
    /// the store), and the files later ones replaced.
    pub parts: Vec<Used>,
    pub replaced: Vec<Replaced>,
}

pub struct Used {
    pub part: Part,
    /// The part's own manifest.
    pub manifest: Manifest,
    pub size: u64,
    pub remote: bool,
}

pub struct Replaced {
    pub path: String,
    pub was: String,
    pub by: String,
}

/// A whole reader read as bytes: for a part that's itself a set.
struct ReaderAt(Arc<Reader>);

impl ReadAt for ReaderAt {
    fn read_exact_at(&self, buf: &mut [u8], off: u64) -> io::Result<()> {
        self.0.read_at(buf, off)
    }
}

/// Files laid one after another: (where it starts, its size, its bytes).
struct Spans(Vec<(u64, u64, Box<dyn ReadAt>)>);

impl Spans {
    fn find(&self, off: u64) -> Option<usize> {
        let i = self.0.partition_point(|s| s.0 <= off).checked_sub(1)?;
        (off < self.0[i].0 + self.0[i].1).then_some(i)
    }
}

impl ReadAt for Spans {
    fn read_exact_at(&self, buf: &mut [u8], off: u64) -> io::Result<()> {
        let Some(i) = self.find(off) else { return Err(io::ErrorKind::UnexpectedEof.into()) };
        let (base, size, r) = &self.0[i];
        if off + buf.len() as u64 > base + size {
            // (Chunks never cross from one part to the next.)
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        r.read_exact_at(buf, off - base)
    }
}

/// Fetching ahead across parts from registries: each part's ranges to its
/// own registry, all at once (a round trip, not one per part).
struct SpansPrefetch(Vec<(u64, u64, Arc<dyn Prefetch>)>);

impl Prefetch for SpansPrefetch {
    fn prefetch(&self, ranges: &[(u64, u64)]) {
        let mut per: Vec<Vec<(u64, u64)>> = vec![Vec::new(); self.0.len()];
        for &(off, len) in ranges {
            for (i, (base, size, _)) in self.0.iter().enumerate() {
                let (start, end) = (off.max(*base), (off + len).min(base + size));
                if start < end {
                    per[i].push((start - base, end - start));
                }
            }
        }
        std::thread::scope(|s| {
            for (i, r) in per.iter().enumerate().filter(|(_, r)| !r.is_empty()) {
                let p = &self.0[i].2;
                s.spawn(move || p.prefetch(r));
            }
        });
    }

    fn stats(&self) -> Stats {
        let mut t = Stats::default();
        for (_, _, p) in &self.0 {
            let s = p.stats();
            t.fetched += s.fetched;
            t.requests += s.requests;
            t.size += s.size;
            t.cached += s.cached;
        }
        t
    }
}

/// One part, opened and (if it's a set) put together, ready to merge.
struct Layer {
    part: Part,
    rd: Arc<Reader>,
    remote: Option<Arc<dyn Prefetch>>,
    from_registry: bool,
    key: String,
}

/// Puts a file together with its parts. A file with none (and no runtime to
/// find) is returned as it is.
pub fn assemble(rd: Reader, remote: Option<Arc<PieceReader>>) -> Result<Assembled> {
    assemble_at(rd, remote.map(|r| r as Arc<dyn Prefetch>), 0)
}

fn assemble_at(rd: Reader, remote: Option<Arc<dyn Prefetch>>, depth: usize) -> Result<Assembled> {
    if depth > MAX_DEPTH {
        bail!("sets inside sets more than {MAX_DEPTH} deep");
    }
    let mut m = rd.manifest.clone();
    let mut wanted: Vec<Part> = m.parts.clone();
    // A runtime pinned by digest: the set's part with it, else the store's.
    // (Only for what's run: a part's runtime is its set's to give.)
    let mut runtime_found = false;
    if let Some(rt) = m.runtime.as_ref().filter(|r| !r.included && !r.digest.is_empty() && depth == 0) {
        if wanted.iter().any(|p| p.role == "runtime" && same_digest(&p.digest, &rt.digest)) {
            runtime_found = true;
        } else {
            let part =
                Part { name: rt.name.clone(), digest: rt.digest.clone(), role: "runtime".into(), from: String::new() };
            if part.digest_bytes().is_some_and(|d| store_path(&d).is_ok_and(|p| p.exists())) {
                wanted.push(part);
                runtime_found = true;
            }
        }
    }
    if wanted.is_empty() {
        let key = hex::encode(rd.digest);
        return Ok(Assembled { rd: Arc::new(rd), remote, key, parts: Vec::new(), replaced: Vec::new() });
    }

    // (All at once: from a registry, each part costs round trips to open.)
    let opened: Vec<Result<Layer>> = std::thread::scope(|s| {
        let m = &m;
        let hs: Vec<_> = wanted
            .iter()
            .map(|p| {
                s.spawn(move || -> Result<Layer> {
                    let o = open_part(p)?;
                    check_fits(m, &o.rd.manifest, p)?;
                    let from_registry = o.remote.is_some();
                    let a = assemble_at(o.rd, o.remote.map(|r| r as Arc<dyn Prefetch>), depth + 1)
                        .with_context(|| format!("part {}", p.name))?;
                    Ok(Layer { part: p.clone(), rd: a.rd, remote: a.remote, from_registry, key: a.key })
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap_or_else(|_| Err(anyhow!("opening a part panicked")))).collect()
    });
    let layers = opened.into_iter().collect::<Result<Vec<Layer>>>()?;

    // One address space: the set's file first, then each part's.
    let set = Arc::new(rd);
    let mut spans: Vec<(u64, u64, Box<dyn ReadAt>)> = vec![(0, set.size(), Box::new(ReaderAt(set.clone())))];
    let mut pre: Vec<(u64, u64, Arc<dyn Prefetch>)> = Vec::new();
    if let Some(r) = &remote {
        pre.push((0, set.size(), r.clone()));
    }
    let mut base = set.size();
    let mut bases = Vec::new();
    for l in &layers {
        bases.push(base);
        spans.push((base, l.rd.size(), Box::new(ReaderAt(l.rd.clone()))));
        if let Some(r) = &l.remote {
            pre.push((base, l.rd.size(), r.clone()));
        }
        base += l.rd.size();
    }

    let mut tree = Tree::default();
    for (l, &b) in layers.iter().zip(&bases) {
        if l.part.role.is_empty() {
            let prefix = format!("parts/{}/", l.part.name);
            tree.put(dir("parts"), "(set)");
            tree.put(dir(&prefix[..prefix.len() - 1]), "(set)");
            for e in &l.rd.entries {
                tree.put(shifted(e, b, &prefix), &l.part.name);
            }
        } else {
            for e in &l.rd.entries {
                tree.put(shifted(e, b, ""), &l.part.name);
            }
        }
    }
    for e in &set.entries {
        tree.put(e.clone(), "(set)");
    }
    if runtime_found {
        if let Some(rt) = m.runtime.as_mut() {
            // (Its files are in runtime/ now, as an included one's are.)
            rt.included = true;
        }
    }
    let entries: Vec<Entry> = tree.entries.into_values().map(|(e, _)| e).collect();
    let all = Reader::assembled(Spans(spans), base, m, set.manifest_json.clone(), entries, set.digest)
        .map_err(|e| anyhow!("putting {} together: {e}", set.manifest.name))?;

    let mut h = Sha256::new();
    h.update(set.digest);
    for l in &layers {
        h.update(l.key.as_bytes());
        h.update(l.part.role.as_bytes());
        h.update(l.part.name.as_bytes());
    }
    let key = format!("{}-{}", hex::encode(set.digest), &hex::encode(h.finalize())[..16]);
    let parts = layers
        .iter()
        .map(|l| Used {
            part: l.part.clone(),
            manifest: l.rd.manifest.clone(),
            size: l.rd.size(),
            remote: l.from_registry,
        })
        .collect();
    let remote: Option<Arc<dyn Prefetch>> = (!pre.is_empty()).then(|| Arc::new(SpansPrefetch(pre)) as _);
    Ok(Assembled { rd: Arc::new(all), remote, key, parts, replaced: tree.replaced })
}

fn same_digest(a: &str, b: &str) -> bool {
    a.strip_prefix("sha256:").unwrap_or(a) == b.strip_prefix("sha256:").unwrap_or(b)
}

fn dir(path: &str) -> Entry {
    let mut e = Entry::new(path, Kind::Dir);
    e.mode = 0o755;
    e
}

fn shifted(e: &Entry, base: u64, prefix: &str) -> Entry {
    let mut e = e.clone();
    if !prefix.is_empty() {
        e.path = format!("{prefix}{}", e.path);
    }
    for c in &mut e.chunks {
        c.offset += base;
    }
    e
}

/// A part can join a set: its platform is the set's (or any), and a runtime
/// part runs what the set needs.
fn check_fits(set: &Manifest, part: &Manifest, p: &Part) -> Result<()> {
    if let (Some(a), Some(b)) = (&set.platform, &part.platform) {
        let any = |s: &str| s == "any" || s.is_empty();
        if !(any(&a.os) || any(&b.os) || a.os == b.os) || !(any(&a.arch) || any(&b.arch) || a.arch == b.arch) {
            bail!("part {} is for {}/{}, the set for {}/{}", p.name, b.os, b.arch, a.os, a.arch);
        }
    }
    if p.role == "runtime" {
        if let Some(rt) = &set.runtime {
            let gives = part.provides.iter().find(|g| g.plug == "runtime");
            match gives {
                Some(g) if g.name == rt.name => {
                    if !rt.version.is_empty() && !crate::run::version_matches(&g.version, &rt.version) {
                        bail!("part {} is {} {}; it runs on {} {}", p.name, g.name, g.version, rt.name, rt.version);
                    }
                }
                Some(g) => bail!("part {} provides the runtime {}, not {}", p.name, g.name, rt.name),
                None => bail!("part {} isn't a runtime (it provides none)", p.name),
            }
        }
    }
    Ok(())
}

/// The merged tree: a later entry replaces an earlier one at its path (and
/// anything under it, when a folder becomes something else); two folders
/// merge.
#[derive(Default)]
struct Tree {
    entries: BTreeMap<String, (Entry, String)>,
    replaced: Vec<Replaced>,
}

impl Tree {
    fn put(&mut self, e: Entry, by: &str) {
        // (A file or link where this needs a folder goes.)
        let mut at = e.path.as_str();
        while let Some(i) = at.rfind('/') {
            at = &at[..i];
            if let Some((old, was)) = self.entries.get(at) {
                if old.kind != Kind::Dir {
                    let was = was.clone();
                    self.entries.remove(at);
                    self.replaced.push(Replaced { path: at.to_string(), was, by: by.into() });
                }
            }
        }
        if let Some((old, was)) = self.entries.get(&e.path) {
            if old.kind == Kind::Dir && e.kind == Kind::Dir {
                let was = was.clone();
                self.entries.insert(e.path.clone(), (e, was));
                return;
            }
            if was != by {
                self.replaced.push(Replaced { path: e.path.clone(), was: was.clone(), by: by.into() });
            }
            if old.kind == Kind::Dir {
                let under = format!("{}/", e.path);
                let gone: Vec<String> = self
                    .entries
                    .range(under.clone()..)
                    .take_while(|(k, _)| k.starts_with(&under))
                    .map(|(k, _)| k.clone())
                    .collect();
                for k in gone {
                    self.entries.remove(&k);
                }
            }
        }
        self.entries.insert(e.path.clone(), (e, by.into()));
    }
}

// --- odi part, odi set, odi runtime ------------------------------------------------

/// A file named on the command line, or a name in a registry: its digest,
/// manifest, and where to get it later.
pub struct Source {
    pub rd: Reader,
    pub from: String,
}

pub fn source(s: &str) -> Result<Source> {
    if Path::new(s).exists() {
        store_add(Path::new(s))?;
        return Ok(Source { rd: Reader::open(s)?, from: String::new() });
    }
    let (rd, _) = crate::hub::open_any(s)?;
    Ok(Source { rd, from: repo_of(s).to_string() })
}

pub fn part_of(name: &str, role: &str, s: &Source) -> Part {
    Part { name: name.into(), digest: digest_string(&s.rd.digest), role: role.into(), from: s.from.clone() }
}

/// odi set: an ODI file of parts, each pinned by digest. It holds no files of
/// its own: the main part's manifest says how to run it.
pub fn cmd_set(args: &[String]) -> Result<()> {
    use crate::flags::{self, flag, Kind::*};
    const U: &str = "odi set MAIN [-o FILE] [--name NAME] [--runtime PART] [--overlay PART]... [--part NAME=PART]...\n  (PART: an ODI file, or a name in a registry)";
    let specs = [
        flag("o", Value, "the set to write (default: NAME.set.odi)"),
        flag("name", Value, "the set's name (default: the main part's)"),
        flag("runtime", Value, "the runtime part (default: the one main pins, if it's in the store)"),
        flag("overlay", Multi, "a part merged over main (repeat; later ones win)"),
        flag("part", Multi, "a part under parts/NAME/ (repeat)"),
    ];
    let (f, pos) = flags::parse(&specs, args, U)?;
    if pos.len() != 1 {
        bail!("usage: {U}");
    }
    let main = source(&pos[0])?;
    let mm = &main.rd.manifest;
    if !mm.parts.is_empty() {
        bail!("{} is a set already: give its parts", pos[0]);
    }
    let mut m = Manifest {
        odi: "0.2".into(),
        name: f.str("name").unwrap_or(&mm.name).to_string(),
        version: mm.version.clone(),
        platform: mm.platform.clone(),
        language: mm.language.clone(),
        runtime: mm.runtime.clone(),
        processes: mm.processes.clone(),
        r#static: mm.r#static.clone(),
        writable: mm.writable.clone(),
        env: mm.env.clone(),
        resources: mm.resources.clone(),
        build: mm.build.clone(),
        needs: mm.needs.clone(),
        provides: mm.provides.clone(),
        parts: vec![part_of(&mm.name, "main", &main)],
    };
    // The runtime: given, or the one main pins (from the store).
    let rt_src = match f.str("runtime") {
        Some(r) => Some(source(r)?),
        None => match m.runtime.as_ref().filter(|r| !r.included && !r.digest.is_empty()) {
            Some(rt) => {
                let d = Part { digest: rt.digest.clone(), ..Default::default() }.digest_bytes();
                match d.map(|d| store_path(&d)).transpose()? {
                    Some(p) if p.exists() => Some(Source { rd: Reader::open(&p)?, from: String::new() }),
                    _ => None,
                }
            }
            None => None,
        },
    };
    if let Some(src) = rt_src {
        let gives = src.rd.manifest.provides.iter().find(|g| g.plug == "runtime").cloned();
        let Some(gives) = gives else { bail!("{} isn't a runtime part (it provides none)", src.rd.manifest.name) };
        match m.runtime.as_mut() {
            Some(rt) if rt.included => bail!("{} includes its runtime already", mm.name),
            Some(rt) => {
                if rt.name != gives.name {
                    bail!("{} runs on {}; the runtime part is {}", mm.name, rt.name, gives.name);
                }
                if !rt.version.is_empty() && !crate::run::version_matches(&gives.version, &rt.version) {
                    bail!("{} runs on {} {}; the runtime part is {}", mm.name, rt.name, rt.version, gives.version);
                }
                rt.version = gives.version.clone();
                rt.digest = digest_string(&src.rd.digest);
            }
            None => {
                m.runtime = Some(odi_format::Runtime {
                    name: gives.name.clone(),
                    version: gives.version.clone(),
                    included: false,
                    digest: digest_string(&src.rd.digest),
                })
            }
        }
        let name = unique(&m, &gives.name);
        m.parts.push(part_of(&name, "runtime", &src));
    }
    for o in f.all("overlay") {
        let src = source(o)?;
        let name = unique(&m, &src.rd.manifest.name);
        m.parts.push(part_of(&name, "overlay", &src));
    }
    for p in f.all("part") {
        let Some((name, at)) = p.split_once('=') else { bail!("--part {p:?}: NAME=PART") };
        if m.parts.iter().any(|q| q.name == name) {
            bail!("--part {name}: a part has that name already");
        }
        m.parts.push(part_of(name, "", &source(at)?));
    }
    m.check().map_err(|e| anyhow!("{e}"))?;
    let out = f.str("o").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("{}.set.odi", m.name)));
    let d = crate::write_odi(&out, &mut m, |_| Ok(()))?;
    // (Put together once now, so a set that can't be is refused here.)
    let a = assemble(Reader::open(&out)?, None)?;
    println!("{} {}", out.display(), digest_string(&d));
    for u in &a.parts {
        let role = if u.part.role.is_empty() { "part" } else { u.part.role.as_str() };
        let from = if u.part.from.is_empty() { String::new() } else { format!("  from {}", u.part.from) };
        println!(
            "  {role:<8} {:<16} {}  {}{from}",
            u.part.name,
            crate::run::short(&u.part.digest),
            crate::human(u.size)
        );
    }
    if !a.replaced.is_empty() {
        println!("  {} files replaced by later parts (odi inspect lists them)", a.replaced.len());
    }
    if a.parts.iter().any(|u| u.part.from.is_empty()) {
        println!("  (parts from files are in this machine's store; to run the set elsewhere, push them and name them from the registry)");
    }
    Ok(())
}

fn unique(m: &Manifest, want: &str) -> String {
    let base = if odi_format::valid_name(want) { want.to_string() } else { "part".into() };
    let mut name = base.clone();
    let mut n = 2;
    while m.parts.iter().any(|p| p.name == name) {
        name = format!("{base}-{n}");
        n += 1;
    }
    name
}

/// odi runtime: a runtime part, made from this machine's runtime (node) or
/// from a folder laid out as one (bin/ first on PATH).
pub fn cmd_runtime(args: &[String]) -> Result<()> {
    use crate::flags::{self, flag, Kind::*};
    const U: &str = "odi runtime node [VERSION] | NAME VERSION DIR  [-o FILE] [--platform OS/ARCH]";
    let specs = [
        flag("o", Value, "the ODI file (default: NAME-VERSION-OS-ARCH.odi)"),
        flag("platform", Value, "OS/ARCH it's for (default: this machine)"),
    ];
    let (f, pos) = flags::parse(&specs, args, U)?;
    // (Fetched by every host that runs one: worth the slower write.)
    crate::set_compress(Some("max"))?;
    let stage = tempfile::Builder::new().prefix("odi-runtime-").tempdir()?;
    let rt = stage.path().join("runtime");
    let (name, version, plat) = match pos.as_slice() {
        [n] | [n, _] if n == "node" => {
            let v = match pos.get(1) {
                Some(v) => v.trim_start_matches('v').to_string(),
                None => host_node_version()?,
            };
            let plat = f.str("platform").map(String::from);
            let plat = plat.unwrap_or_else(|| format!("{}/{}", crate::host_os(), crate::host_arch()));
            node_release(&v, &plat, &rt)?;
            ("node".to_string(), v, plat)
        }
        [n, v, d] => {
            if !odi_format::valid_name(n) {
                bail!("{n}: a runtime's name is letters, digits, . - _");
            }
            if !Path::new(d).join("bin").is_dir() {
                bail!("{d}: no bin/ folder (a runtime's executables are in runtime/bin/)");
            }
            crate::copy_tree(Path::new(d), &rt)?;
            let plat = f.str("platform").map(String::from);
            (n.clone(), v.clone(), plat.unwrap_or_else(|| format!("{}/{}", crate::host_os(), crate::host_arch())))
        }
        _ => bail!("usage: {U}"),
    };
    let Some((os, arch)) = plat.split_once('/') else { bail!("--platform: OS/ARCH") };
    // (nodejs.org's Linux builds are for glibc.)
    let libc = if os == "linux" && name == "node" && pos.len() <= 2 { "gnu" } else { "" };
    let mut m = Manifest {
        odi: "0.2".into(),
        name: name.clone(),
        version: version.clone(),
        platform: Some(odi_format::Platform { os: os.into(), arch: arch.into(), libc: libc.into() }),
        provides: vec![odi_format::Plug {
            plug: "runtime".into(),
            name: name.clone(),
            version: version.clone(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let out =
        f.str("o").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("{name}-{version}-{os}-{arch}.odi")));
    let d = crate::pack(stage.path(), &mut m, &out)?;
    store_add(&out)?;
    println!("{} {}  ({} {} for {plat}; in this machine's store)", out.display(), digest_string(&d), name, version);
    Ok(())
}

/// odi part: this machine's store of parts.
pub fn cmd_part(args: &[String]) -> Result<()> {
    const U: &str = "odi part add FILE... | odi part ls";
    match args.split_first() {
        Some((c, files)) if c == "add" && !files.is_empty() => {
            for f in files {
                let d = store_add(Path::new(f))?;
                println!("{f} {}", digest_string(&d));
            }
            Ok(())
        }
        Some((c, rest)) if c == "ls" && rest.is_empty() => {
            let dir = store_dir()?;
            let Ok(rd) = fs::read_dir(&dir) else { return Ok(()) };
            let mut names: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
            names.sort();
            for p in names.iter().filter(|p| p.extension().is_some_and(|e| e == "odi")) {
                match Reader::open(p) {
                    Ok(r) => {
                        let m = &r.manifest;
                        let gives: Vec<String> =
                            m.provides.iter().map(|g| format!("{} {} {}", g.plug, g.name, g.version)).collect();
                        println!(
                            "sha256:{}  {} {}  {}  {}",
                            &hex::encode(r.digest)[..12],
                            m.name,
                            m.version,
                            crate::human(r.size()),
                            gives.join(", ")
                        );
                    }
                    Err(e) => println!("{}: {e}", p.display()),
                }
            }
            Ok(())
        }
        _ => bail!("usage: {U}"),
    }
}

/// The node on this machine's PATH: its version.
fn host_node_version() -> Result<String> {
    let out =
        std::process::Command::new("node").arg("--version").output().context("node --version (or give a version)")?;
    let v = String::from_utf8_lossy(&out.stdout).trim().trim_start_matches('v').to_string();
    if v.is_empty() {
        bail!("node --version said nothing: give a version");
    }
    Ok(v)
}

/// Node's own release for a platform, from nodejs.org, checked against its
/// published SHA-256, laid out in dir as a runtime (bin/, lib/). The same
/// release makes the same part, with the same digest, wherever it's made.
fn node_release(v: &str, plat: &str, dir: &Path) -> Result<()> {
    let Some((os, arch)) = plat.split_once('/') else { bail!("--platform: OS/ARCH") };
    let arch = match arch {
        "amd64" => "x64",
        "arm64" => "arm64",
        a => bail!("node for {os}/{a}: nodejs.org has builds for amd64 and arm64"),
    };
    if !matches!(os, "linux" | "darwin") {
        bail!("node for {os}: linux or darwin");
    }
    let base = std::env::var("ODI_NODE_MIRROR").unwrap_or_else(|_| "https://nodejs.org/dist".into());
    let name = format!("node-v{v}-{os}-{arch}");
    let file = format!("{name}.tar.gz");
    let get = |url: &str| -> Result<Vec<u8>> {
        let res = ureq::get(url).call().with_context(|| url.to_string())?;
        let mut b = Vec::new();
        io::Read::read_to_end(&mut res.into_reader().take(1 << 30), &mut b)?;
        Ok(b)
    };
    let sums = String::from_utf8(get(&format!("{base}/v{v}/SHASUMS256.txt"))?)?;
    let want = sums
        .lines()
        .find_map(|l| l.split_once("  ").filter(|(_, f)| *f == file).map(|(h, _)| h.to_string()))
        .ok_or_else(|| anyhow!("node {v}: no {file} in its SHASUMS256.txt"))?;
    eprintln!("== {base}/v{v}/{file}");
    let tgz = get(&format!("{base}/v{v}/{file}"))?;
    if hex::encode(Sha256::digest(&tgz)) != want {
        bail!("{file}: its SHA-256 isn't the one nodejs.org publishes");
    }
    let x = dir.parent().expect("in the stage").join("x");
    fs::create_dir_all(&x)?;
    fs::write(x.join(&file), &tgz)?;
    drop(tgz);
    let st = std::process::Command::new("tar").arg("xzf").arg(&file).current_dir(&x).status()?;
    if !st.success() {
        bail!("tar xzf {file}: {st}");
    }
    let src = x.join(&name);
    fs::create_dir_all(dir)?;
    for keep in ["bin", "lib", "LICENSE"] {
        if src.join(keep).exists() {
            crate::copy_tree(&src.join(keep), &dir.join(keep))?;
        }
    }
    fs::remove_dir_all(&x)?;
    Ok(())
}

/// odi fill: gets what a file will read ahead of its first run, into this
/// machine's piece cache (and its parts' too): its start list, or all of it.
/// Run from the registry afterwards, it reads from local disk.
pub fn cmd_fill(args: &[String]) -> Result<()> {
    use crate::flags::{self, flag, Kind::*};
    const U: &str = "odi fill NAME|FILE... [--all]";
    let specs = [flag("all", Bool, "all of it, not just its start list")];
    let (f, pos) = flags::parse(&specs, args, U)?;
    if pos.is_empty() {
        bail!("usage: {U}");
    }
    for name in &pos {
        let t = std::time::Instant::now();
        let (rd, remote) = crate::hub::open_any(name)?;
        if remote.is_none() {
            // (A file here: it and the parts it names go in the store.)
            store_add(Path::new(name))?;
        }
        let digest = digest_string(&rd.digest);
        let a = assemble(rd, remote)?;
        let Some(pf) = &a.remote else {
            println!("{name} {digest}: all of it is on this machine");
            continue;
        };
        let start = crate::hints::load::<crate::hints::StartList>(&a.rd, crate::hints::START);
        let (what, ranges) = match start.filter(|_| !f.bool("all")) {
            Some(s) => ("its start list", s.ranges(&a.rd)),
            None => ("all of it", vec![(0, a.rd.size())]),
        };
        let before = pf.stats();
        pf.prefetch(&ranges);
        let s = pf.stats();
        println!(
            "{name} {digest}: {what}, {} fetched in {} requests, {} here already ({} ms)",
            crate::human(s.fetched - before.fetched),
            s.requests - before.requests,
            crate::human(s.cached),
            t.elapsed().as_millis()
        );
    }
    Ok(())
}
