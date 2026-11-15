//! odi builds, checks and runs ODI files (SPEC.md): one file holding a
//! program's build output, for any language, that runs anywhere.

mod build;
mod cli;
mod cross;
mod flags;
mod hints;
mod hub;
mod keys;
mod mount;
mod parts;
mod run;
mod web;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use odi_format::{digest_string, Kind, Manifest, Reader, Writer, TOP_LEVEL};

use crate::flags::{flag, Kind::*};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    // (A closed pipe ends odi quietly, as it does any Unix tool: `odi inspect
    // x | head`. Rust ignores SIGPIPE by default, and printing then panics.)
    #[cfg(unix)]
    // SAFETY: called first, before any other thread exists.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        eprint!("{}", cli::overview());
        std::process::exit(2);
    };
    let rest = &args[1..];
    // (--help for commands that take no flags; the others answer it where
    // they parse theirs, with them.)
    let wants_help = rest.iter().take_while(|a| *a != "--").any(|a| a == "-h" || a == "--help");
    if wants_help {
        if let Some(c) = cli::find(cmd).filter(|c| !c.flags) {
            print!("{}", cli::command_help(c, &[]));
            return;
        }
    }
    let r = match cmd.as_str() {
        "build" => build::cmd_build(rest),
        "pack" => cmd_pack(rest),
        "run" => run::cmd_run(rest),
        "profile" => run::cmd_profile(rest),
        "set" => parts::cmd_set(rest),
        "runtime" => parts::cmd_runtime(rest),
        "part" => parts::cmd_part(rest),
        "fill" => parts::cmd_fill(rest),
        "exec" => run::cmd_exec(rest),
        "mcp" => run::cmd_mcp(rest),
        "attach" => run::cmd_attach(rest),
        "web" => web::cmd_web(rest),
        "key" => keys::cmd_key(rest),
        "sign" => keys::cmd_sign(rest),
        "inspect" => cmd_inspect(rest),
        "verify" => cmd_verify(rest),
        "extract" => cmd_extract(rest),
        "login" => hub::cmd_login(rest),
        "logout" => hub::cmd_logout(),
        "push" => hub::cmd_push(rest),
        "pull" => hub::cmd_pull(rest),
        "tags" => hub::cmd_tags(rest),
        "rm" => hub::cmd_rm(rest),
        "usage" => hub::cmd_usage(rest),
        "version" | "--version" | "-V" => {
            println!("{}", version_line());
            Ok(())
        }
        "help" | "-h" | "--help" => cmd_help(rest),
        "completions" => match rest {
            [shell] => match cli::completions(shell) {
                Some(s) => {
                    print!("{s}");
                    Ok(())
                }
                None => Err(anyhow!("no completions for {shell:?}: bash, zsh or fish")),
            },
            _ => Err(anyhow!("usage: odi completions bash|zsh|fish")),
        },
        other => {
            let hint = match cli::suggest(other) {
                Some(s) => format!(" (did you mean odi {s}?)"),
                None => String::new(),
            };
            eprintln!("odi: unknown command {other:?}{hint}\nRun 'odi help' for the commands.");
            std::process::exit(2);
        }
    };
    if let Err(e) = r {
        eprintln!("odi: {e:#}");
        std::process::exit(1);
    }
}

/// odi's version, the commit it was built from (when the release build
/// says), and the machine it's for.
fn version_line() -> String {
    let commit = option_env!("ODI_COMMIT").map(|c| format!(" ({})", &c[..c.len().min(12)])).unwrap_or_default();
    format!("odi {VERSION}{commit} {}/{}", host_os(), host_arch())
}

fn cmd_help(args: &[String]) -> Result<()> {
    match args {
        [] => {
            print!("{}", cli::overview());
            Ok(())
        }
        // (For docs/gen-cli.sh: every command's name.)
        [l] if l == "--list" => {
            for c in cli::COMMANDS {
                println!("{}", c.name);
            }
            Ok(())
        }
        [name] => match cli::find(name) {
            Some(c) if c.flags => {
                // Its flags are where it parses them: it prints its help.
                let exe = std::env::current_exe().context("odi's own path")?;
                let st = std::process::Command::new(exe).args([name.as_str(), "--help"]).status()?;
                std::process::exit(st.code().unwrap_or(1));
            }
            Some(c) => {
                print!("{}", cli::command_help(c, &[]));
                Ok(())
            }
            None => {
                let hint = cli::suggest(name).map(|s| format!(" (did you mean {s}?)")).unwrap_or_default();
                bail!("no command {name:?}{hint}: odi help lists them")
            }
        },
        _ => bail!("usage: odi help [COMMAND]"),
    }
}

// --- this machine ---------------------------------------------------------------

/// This machine's OS, as ODI (and Go) names it.
pub fn host_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        o => o,
    }
}

/// This machine's CPU, as ODI (and Go) names it.
pub fn host_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "x86" => "386",
        a => a,
    }
}

// --- folders --------------------------------------------------------------------

pub enum Step {
    Continue,
    SkipDir,
}

/// Walks a folder in name order, root first, without following links (as Go's
/// filepath.WalkDir). f gets each path and its own metadata.
pub fn walk(root: &Path, f: &mut dyn FnMut(&Path, &fs::Metadata) -> Result<Step>) -> Result<()> {
    let meta = fs::symlink_metadata(root).with_context(|| format!("{}", root.display()))?;
    if let Step::SkipDir = f(root, &meta)? {
        return Ok(());
    }
    if meta.is_dir() {
        let mut names: Vec<PathBuf> = fs::read_dir(root)
            .with_context(|| format!("{}", root.display()))?
            .map(|e| e.map(|e| e.path()))
            .collect::<io::Result<_>>()?;
        names.sort();
        for p in names {
            walk(&p, f)?;
        }
    }
    Ok(())
}

/// A path under root, `/`-separated, as the index writes it.
pub fn rel_slash(root: &Path, p: &Path) -> Result<String> {
    let rel = p.strip_prefix(root).unwrap_or(p);
    let s = rel.to_str().ok_or_else(|| anyhow!("{}: a name that isn't UTF-8", p.display()))?;
    Ok(s.replace(std::path::MAIN_SEPARATOR, "/"))
}

#[cfg(unix)]
pub fn mode_of(m: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    m.permissions().mode() & 0o777
}

#[cfg(not(unix))]
pub fn mode_of(m: &fs::Metadata) -> u32 {
    if m.is_dir() {
        0o755
    } else if m.permissions().readonly() {
        0o444
    } else {
        0o644
    }
}

#[cfg(unix)]
pub fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
pub fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[cfg(unix)]
pub fn set_mode(p: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(p, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
pub fn set_mode(_: &Path, _: u32) -> io::Result<()> {
    Ok(())
}

/// Copies a folder (files, folders, symlinks as they are), or one file.
pub fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    walk(src, &mut |p, meta| {
        // (Not dst.join(""): that's dst with a trailing slash, no name for a file.)
        let to = match p.strip_prefix(src) {
            Ok(rel) if !rel.as_os_str().is_empty() => dst.join(rel),
            _ => dst.to_path_buf(),
        };
        copy_one(p, &to, meta)?;
        Ok(Step::Continue)
    })
}

/// Copies one folder (made, not filled), link or file; anything else is skipped.
pub fn copy_one(p: &Path, to: &Path, meta: &fs::Metadata) -> Result<()> {
    let ft = meta.file_type();
    if ft.is_dir() {
        fs::create_dir_all(to)?;
        set_mode(to, mode_of(meta) | 0o700)?;
    } else if ft.is_symlink() {
        symlink(&fs::read_link(p)?, to).with_context(|| format!("{}", to.display()))?;
    } else if ft.is_file() {
        fs::copy(p, to).with_context(|| format!("{}", p.display()))?;
        set_mode(to, mode_of(meta))?;
    }
    Ok(())
}

pub fn exists(dir: &Path, name: &str) -> bool {
    dir.join(name).exists()
}

// --- pack -----------------------------------------------------------------------

/// Writes the folder root (holding app/, static/, runtime/, meta/) and the
/// manifest into out; returns the digest.
pub fn pack(root: &Path, m: &mut Manifest, out: &Path) -> Result<[u8; 32]> {
    write_odi(out, m, |w| {
        // (Runner hints are written last, next to the index, so a reader from
        // a registry gets them in the same request as the index.)
        let mut hints = Vec::new();
        walk(root, &mut |p, meta| {
            if p == root {
                return Ok(Step::Continue);
            }
            let rel = rel_slash(root, p)?;
            let top = rel.split('/').next().unwrap_or("");
            if !TOP_LEVEL.contains(&top) {
                // (only app/, static/, runtime/, meta/ go in)
                return Ok(Step::SkipDir);
            }
            let ft = meta.file_type();
            if ft.is_dir() {
                w.add_dir(&rel, mode_of(meta))?;
            } else if ft.is_symlink() {
                let t = fs::read_link(p)?;
                let t = t.to_str().ok_or_else(|| anyhow!("{}: a link to a name that isn't UTF-8", p.display()))?;
                w.add_symlink(&rel, &t.replace(std::path::MAIN_SEPARATOR, "/"))?;
            } else if ft.is_file() && rel.starts_with("meta/odi/") {
                hints.push((rel, p.to_path_buf(), mode_of(meta)));
            } else if ft.is_file() {
                let r = fs::File::open(p).with_context(|| format!("{}", p.display()))?;
                w.add_file(&rel, mode_of(meta), r)?;
            } // (sockets, devices: not build output)
            Ok(Step::Continue)
        })?;
        for (rel, p, mode) in hints {
            w.add_file(&rel, mode, fs::File::open(&p)?)?;
        }
        Ok(())
    })
}

/// A file entry's content, read chunk by chunk (each checked).
struct EntryReader<'a> {
    rd: &'a Reader,
    e: &'a odi_format::Entry,
    next: usize,
    buf: Vec<u8>,
    pos: usize,
}

impl io::Read for EntryReader<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.pos == self.buf.len() {
            let Some(c) = self.e.chunks.get(self.next) else { return Ok(0) };
            self.buf = self.rd.read_chunk(c).map_err(io::Error::other)?;
            self.next += 1;
            self.pos = 0;
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// Writes rd again into out, with files added or replaced (a runner hint,
/// say). The manifest is written from what this odi knows of it: fields it
/// doesn't know aren't kept.
pub fn repack(rd: &Reader, add: &[(&str, Vec<u8>)], out: &Path) -> Result<[u8; 32]> {
    let mut m = rd.manifest.clone();
    write_odi(out, &mut m, |w| {
        let hint = |e: &&odi_format::Entry| e.path.starts_with("meta/odi/");
        let kept: Vec<&odi_format::Entry> =
            rd.entries.iter().filter(|e| !add.iter().any(|(p, _)| *p == e.path)).collect();
        // (Runner hints last, next to the index: see pack.)
        for e in kept.iter().filter(|e| !hint(e)).chain(kept.iter().filter(|e| hint(e))) {
            match e.kind {
                Kind::Dir => w.add_dir(&e.path, e.mode)?,
                Kind::Symlink => w.add_symlink(&e.path, &e.target)?,
                Kind::File => w.add_file(&e.path, e.mode, EntryReader { rd, e, next: 0, buf: Vec::new(), pos: 0 })?,
            }
        }
        for (p, b) in add {
            w.add_file(p, 0o644, &b[..])?;
        }
        Ok(())
    })
}

/// zstd's level for chunks odi writes: 0 is the writer's default; odi build
/// --compress max sets 19 (about 10% smaller, reads as fast, slower to write).
pub static COMPRESS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// --compress default|max.
pub fn set_compress(v: Option<&str>) -> Result<()> {
    let level = match v {
        None | Some("default") => 0,
        Some("max") => 19,
        Some(o) => bail!("--compress {o:?}: default or max"),
    };
    COMPRESS.store(level, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/// Writes an ODI file whole or not at all: to out.tmp, then renamed.
pub fn write_odi(
    out: &Path,
    m: &mut Manifest,
    fill: impl FnOnce(&mut Writer<&mut fs::File>) -> Result<()>,
) -> Result<[u8; 32]> {
    let mut tmp_name = out.as_os_str().to_owned();
    tmp_name.push(".tmp");
    let tmp = PathBuf::from(tmp_name);
    let r = (|| -> Result<[u8; 32]> {
        let mut f = fs::File::create(&tmp).with_context(|| format!("{}", tmp.display()))?;
        let mut w = Writer::new(&mut f)?;
        let level = COMPRESS.load(std::sync::atomic::Ordering::Relaxed);
        if level != 0 {
            w.set_level(level)?;
        }
        fill(&mut w)?;
        let d = w.finish(m)?;
        f.sync_all()?;
        Ok(d)
    })();
    match r {
        Ok(d) => {
            fs::rename(&tmp, out)?;
            Ok(d)
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn cmd_pack(args: &[String]) -> Result<()> {
    const U: &str = "odi pack DIR [-o FILE] [--manifest FILE] [--compress max]";
    let specs = [
        flag("compress", Value, "default, or max (zstd 19: about 10% smaller, as fast to read)"),
        flag("o", Value, "the ODI file to write (default: NAME.odi)"),
        flag("manifest", Value, "the manifest (default: DIR/odi.manifest.json, kept out of the file)"),
    ];
    let (f, pos) = flags::parse(&specs, args, U)?;
    if pos.len() != 1 {
        bail!("usage: {U}");
    }
    set_compress(f.str("compress"))?;
    let dir = Path::new(&pos[0]);
    let mf = f.str("manifest").map(PathBuf::from).unwrap_or_else(|| dir.join("odi.manifest.json"));
    let b = fs::read(&mf).context("the manifest")?;
    let mut m: Manifest = serde_json::from_slice(&b).context("the manifest")?;
    let out = f.str("o").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("{}.odi", m.name)));
    let d = pack(dir, &mut m, &out)?;
    println!("{} {}", out.display(), digest_string(&d));
    Ok(())
}

// --- inspect, verify, extract ----------------------------------------------------

fn cmd_inspect(args: &[String]) -> Result<()> {
    if args.len() != 1 {
        bail!("usage: odi inspect FILE|NAMESPACE/NAME[:TAG]");
    }
    let (rd, remote) = hub::open_any(&args[0])?;
    let file_size = rd.size();
    let m = &rd.manifest.clone();
    println!("{} {}", m.name, m.version);
    println!("  digest    {}", digest_string(&rd.digest));
    for s in &rd.signatures {
        let ok = if s.check(&rd.digest).is_ok() { "" } else { " (doesn't match!)" };
        println!("  signed    {}{ok}", s.key);
    }
    if let Some(p) = &m.platform {
        let mut s = format!("{}/{}", p.os, p.arch);
        if !p.libc.is_empty() {
            s += &format!(" ({})", p.libc);
        }
        println!("  platform  {s}");
    }
    if let Some(l) = &m.language {
        println!("  language  {} {}", l.name, l.framework);
    }
    if let Some(r) = &m.runtime {
        let inc = if r.included { "included" } else { "not included" };
        println!("  runtime   {} {} ({inc})", r.name, r.version);
    }
    for p in &m.processes {
        let port = match &p.port {
            Some(pt) => format!("  port ${} (default {})", or_default(&pt.env, "PORT"), pt.default),
            None => String::new(),
        };
        println!("  process   {}: {}  in {}{port}", p.name, p.command.join(" "), or_default(&p.workdir, "app"));
    }
    for s in &m.r#static {
        println!("  static    {} at {}", s.from, s.at);
    }
    if let Some(env) = m.env.as_ref().filter(|e| !e.required.is_empty()) {
        println!("  needs     {}", env.required.join(", "));
    }
    for g in &m.provides {
        println!("  provides  {} {} {}", g.plug, g.name, g.version);
    }
    for g in &m.needs {
        println!("  needs     {} {} {}", g.plug, g.name, g.version);
    }
    for p in &m.parts {
        let role = if p.role.is_empty() { "(under parts/)" } else { p.role.as_str() };
        let from = if p.from.is_empty() { String::new() } else { format!("  from {}", p.from) };
        println!("  part      {} {}  {role}{from}", p.name, run::short(&p.digest));
    }
    if m.parts.is_empty() && m.runtime.as_ref().is_none_or(|r| r.digest.is_empty() || r.included) {
        return contents(&rd, file_size);
    }
    let a = match parts::assemble(rd, remote) {
        Ok(a) => a,
        Err(e) => {
            println!("  (can't be put together here: {e:#})");
            return Ok(());
        }
    };
    for r in a.replaced.iter().take(20) {
        println!("  replaces  {} (of {}, by {})", r.path, r.was, r.by);
    }
    if a.replaced.len() > 20 {
        println!("  replaces  ... and {} more", a.replaced.len() - 20);
    }
    let whole: u64 = a.parts.iter().map(|u| u.size).sum();
    println!("  together  {} parts, {} in all", a.parts.len(), human(whole + file_size));
    contents(&a.rd, file_size)
}

fn contents(rd: &Reader, file_size: u64) -> Result<()> {
    let (mut files, mut size) = (0u64, 0u64);
    let mut tops = std::collections::HashMap::new();
    for e in rd.entries.iter().filter(|e| e.kind == Kind::File) {
        files += 1;
        size += e.size;
        *tops.entry(e.path.split('/').next().unwrap_or("")).or_insert(0u64) += e.size;
    }
    let parts: Vec<String> =
        TOP_LEVEL.iter().chain(&["parts"]).filter_map(|t| tops.get(t).map(|n| format!("{t}/ {}", human(*n)))).collect();
    println!("  contents  {files} files, {} ({}); the file: {}", human(size), parts.join(", "), human(file_size));
    Ok(())
}

fn cmd_verify(args: &[String]) -> Result<()> {
    const U: &str = "odi verify FILE [--key KEY]...";
    let specs = [flag("key", Multi, "a public key that must have signed it (repeat: any of them)")];
    let (f, pos) = flags::parse(&specs, args, U)?;
    if pos.len() != 1 {
        bail!("usage: {U}");
    }
    let rd = Reader::open(&pos[0])?;
    rd.verify()?;
    for s in &rd.signatures {
        match s.check(&rd.digest) {
            Ok(()) => println!("signed by {}", s.key),
            Err(e) => println!("a bad signature: {e}"),
        }
    }
    if !f.all("key").is_empty() {
        let want = f.all("key");
        for k in want {
            odi_format::parse_key(k).map_err(|e| anyhow!("--key: {e}"))?;
        }
        if !rd.signatures.iter().any(|s| want.contains(&s.key) && s.check(&rd.digest).is_ok()) {
            bail!("not signed by {}", want.join(" or "));
        }
    }
    // (A set: every part it's made of, too, as far as they're here.)
    if !rd.manifest.parts.is_empty() {
        let d = rd.digest;
        let a = parts::assemble(rd, None)?;
        a.rd.verify()?;
        println!("ok {} (and its {} parts)", digest_string(&d), a.parts.len());
        return Ok(());
    }
    println!("ok {}", digest_string(&rd.digest));
    Ok(())
}

fn cmd_extract(args: &[String]) -> Result<()> {
    if args.len() != 2 {
        bail!("usage: odi extract FILE DIR");
    }
    // (A set unpacks put together.)
    let rd = parts::assemble(Reader::open(&args[0])?, None)?.rd;
    let dir = Path::new(&args[1]);
    if let Ok(mut d) = fs::read_dir(dir) {
        if d.next().is_some() {
            bail!("{} isn't empty", dir.display());
        }
    }
    extract(&rd, dir)
}

/// Unpacks every entry under dir (which must be new or empty). Paths were
/// checked when the file was read (§3.4); links are made last, so no file is
/// ever written through one.
pub fn extract(rd: &Reader, dir: &Path) -> Result<()> {
    extract_shared(rd, dir, None)
}

/// As extract, but the runtime's files (runtime/) are hard links into share,
/// one copy per content and mode: every app pinning one runtime part shares
/// its files on disk. (Where links can't be made, they're copied.)
pub fn extract_shared(rd: &Reader, dir: &Path, share: Option<&Path>) -> Result<()> {
    use cap_std::ambient_authority;
    use cap_std::fs::{Dir, OpenOptions};

    fs::create_dir_all(dir)?;
    // (Every write goes through root, which can't leave dir, whatever the file
    // holds. A case-insensitive disk can join names the reader saw as
    // different, so what's there is checked again at the end.)
    let root = Dir::open_ambient_dir(dir, ambient_authority())?;
    let parent_of = |p: &str| p.rfind('/').map(|i| p[..i].to_string());
    let mut links = Vec::new();
    for e in &rd.entries {
        match e.kind {
            Kind::Dir => root.create_dir_all(&e.path).with_context(|| e.path.clone())?,
            Kind::Symlink => links.push(e),
            Kind::File => {
                if let Some(d) = parent_of(&e.path) {
                    root.create_dir_all(&d).with_context(|| e.path.clone())?;
                }
                if let Some(sh) = share.filter(|_| e.path.starts_with("runtime/")) {
                    if link_shared(rd, e, sh, dir)? {
                        continue;
                    }
                }
                let mut opts = OpenOptions::new();
                opts.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use cap_std::fs::OpenOptionsExt;
                    opts.mode(e.mode | 0o200);
                }
                let mut out = root.open_with(&e.path, &opts).with_context(|| e.path.clone())?;
                rd.write_file(e, &mut out)?;
                drop(out);
                chmod(&root, &e.path, e.mode);
            }
        }
    }
    for e in &links {
        if let Some(d) = parent_of(&e.path) {
            root.create_dir_all(&d).with_context(|| e.path.clone())?;
        }
        #[cfg(unix)]
        root.symlink(&e.target, &e.path).with_context(|| e.path.clone())?;
        #[cfg(windows)]
        root.symlink_file(&e.target, &e.path).with_context(|| e.path.clone())?;
    }
    let real = fs::canonicalize(dir)?;
    for e in &links {
        match fs::canonicalize(real.join(&e.path)) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Ok(to) if to.starts_with(&real) => {}
            _ => bail!("{}: its target {:?} points outside", e.path, e.target),
        }
    }
    // Directories' own modes last (a read-only one would stop the writes).
    for e in rd.entries.iter().rev() {
        if e.kind == Kind::Dir && e.mode != 0 {
            chmod(&root, &e.path, e.mode | 0o700);
        }
    }
    Ok(())
}

/// Links a file entry from share (written there first if it's new). False
/// when no link can be made here.
fn link_shared(rd: &Reader, e: &odi_format::Entry, share: &Path, dir: &Path) -> Result<bool> {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    for c in &e.chunks {
        h.update(c.hash.as_bytes());
    }
    h.update(format!(" {:o}", e.mode).as_bytes());
    let key = hex::encode(h.finalize());
    let src = share.join(&key[..2]).join(&key);
    if !src.exists() {
        fs::create_dir_all(src.parent().expect("in a folder"))?;
        let mut tmp = src.clone().into_os_string();
        tmp.push(format!(".tmp-{}", std::process::id()));
        let tmp = PathBuf::from(tmp);
        let mut f = fs::File::create(&tmp)?;
        rd.write_file(e, &mut f)?;
        drop(f);
        set_mode(&tmp, e.mode & 0o555)?;
        fs::rename(&tmp, &src)?;
    }
    // (The path was checked when the file was read, and its folders were made
    // inside dir through root: no link on the way.)
    let to = dir.join(&e.path);
    let canon_parent = fs::canonicalize(to.parent().expect("in a folder"))?;
    if !canon_parent.starts_with(fs::canonicalize(dir)?) {
        bail!("{}: outside", e.path);
    }
    Ok(fs::hard_link(&src, &to).is_ok())
}

#[cfg(unix)]
fn chmod(root: &cap_std::fs::Dir, p: &str, mode: u32) {
    use cap_std::fs::{Permissions, PermissionsExt};
    let _ = root.set_permissions(p, Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn chmod(_: &cap_std::fs::Dir, _: &str, _: u32) {}

pub fn or_default<'a>(s: &'a str, d: &'a str) -> &'a str {
    if s.is_empty() {
        d
    } else {
        s
    }
}

pub fn human(n: u64) -> String {
    let f = n as f64;
    match n {
        n if n >= 1 << 30 => format!("{:.1} GB", f / (1u64 << 30) as f64),
        n if n >= 1 << 20 => format!("{:.1} MB", f / (1u64 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.1} KB", f / (1u64 << 10) as f64),
        n => format!("{n} B"),
    }
}

#[cfg(test)]
mod tests;
