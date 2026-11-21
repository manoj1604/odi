//! One executable that is an app: `odi bundle app.odi -o app` writes a
//! static odi for the target machine with the ODI file after it, unchanged
//! (its digest is the same), then a trailer saying where it is:
//!
//!   [odi, the runner] [the ODI file] [its length: u64 LE] [count: 1, u64 LE]
//!   [ODIEXEC\0]
//!
//! (The count leaves room for more files later. What the app needs, its
//! runtime included, is the ODI file's to carry: odi.json's include_runtime.)
//!
//! Run, the executable finds the file inside itself and runs it as `odi run`
//! would: nothing to install where it runs (a VM, a microVM, a server).

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use odi_format::{ReadAt, Reader};
use sha2::{Digest, Sha256};

use crate::flags::{self, flag, Kind::*};

const MAGIC: &[u8; 8] = b"ODIEXEC\0";
const TRAILER: u64 = 16;
/// In every odi that can run a bundle: how odi bundle tells one from an
/// older odi (which would ignore the file after it).
pub static RUNNER_MARK: &str = "odi-bundle-runner-1";

/// The ODI files after this executable, when it's a bundle: (the
/// executable, each file's (start, length), the app last).
pub fn embedded() -> Option<(fs::File, Vec<(u64, u64)>)> {
    std::hint::black_box(RUNNER_MARK);
    let exe = std::env::current_exe().ok()?;
    let f = fs::File::open(exe).ok()?;
    let size = f.metadata().ok()?.len();
    if size < TRAILER + 200 {
        return None;
    }
    let mut t = [0u8; 16];
    f.read_exact_at(&mut t, size - TRAILER).ok()?;
    if &t[8..] != MAGIC {
        return None;
    }
    let n = u64::from_le_bytes(t[..8].try_into().ok()?);
    if n == 0 || n > 64 {
        return None;
    }
    let lens_at = (size - TRAILER).checked_sub(8 * n)?;
    let mut lens = vec![0u8; 8 * n as usize];
    f.read_exact_at(&mut lens, lens_at).ok()?;
    let lens: Vec<u64> = lens.chunks(8).map(|c| u64::from_le_bytes(c.try_into().unwrap_or_default())).collect();
    let mut end = lens_at;
    let mut out = vec![(0, 0); lens.len()];
    for (i, len) in lens.iter().enumerate().rev() {
        end = end.checked_sub(*len)?;
        out[i] = (end, *len);
    }
    Some((f, out))
}

/// A file's bytes from start, as if they were a file of their own.
struct Slice {
    f: fs::File,
    start: u64,
}

impl ReadAt for Slice {
    fn read_exact_at(&self, buf: &mut [u8], off: u64) -> std::io::Result<()> {
        self.f.read_exact_at(buf, self.start + off)
    }
}

/// Runs the ODI file inside this executable, with odi run's flags.
pub fn run_embedded(f: fs::File, files: Vec<(u64, u64)>, args: &[String]) -> Result<()> {
    let (start, len) = *files.last().expect("at least one");
    let rd = Reader::new(Slice { f: f.try_clone()?, start }, len).context("the app inside this executable")?;
    if args.first().map(String::as_str) == Some("--odi-file") {
        // (the ODI file back out, as it was bundled)
        let [_, out] = args else { bail!("usage: --odi-file OUT.odi") };
        let mut w = fs::File::create(out)?;
        let mut left = len;
        let mut buf = vec![0u8; 1 << 20];
        let mut off = start;
        while left > 0 {
            let n = left.min(buf.len() as u64) as usize;
            f.read_exact_at(&mut buf[..n], off)?;
            w.write_all(&buf[..n])?;
            off += n as u64;
            left -= n as u64;
        }
        eprintln!("{out}: {}", odi_format::digest_string(&rd.digest));
        return Ok(());
    }
    let name = std::env::args().next().unwrap_or_else(|| "app".into());
    crate::run::run_reader(rd, args, &format!("{name} [--port 8080] [-e KEY=VALUE]... [--trust KEY]...   ({name} --odi-file OUT.odi: the ODI file inside)"))
}

/// The Rust target odi's releases are built for, for an ODI platform.
fn triple(os: &str, arch: &str) -> Result<&'static str> {
    Ok(match (os, arch) {
        ("linux", "amd64") => "x86_64-unknown-linux-musl",
        ("linux", "arm64") => "aarch64-unknown-linux-musl",
        ("darwin", "amd64") => "x86_64-apple-darwin",
        ("darwin", "arm64") => "aarch64-apple-darwin",
        _ => bail!("no odi for {os}/{arch} to bundle with (linux or darwin, amd64 or arm64)"),
    })
}

pub fn cmd_bundle(args: &[String]) -> Result<()> {
    const U: &str = "odi bundle FILE [-o OUT] [--platform OS/ARCH] [--runner FILE]";
    let specs = [
        flag("o", Value, "the executable to write (default: the file's name, without .odi)"),
        flag("platform", Value, "the machine it's for, when the file runs anywhere (default: linux and this CPU)"),
        flag("runner", Value, "the odi to bundle with (default: this one if it fits, else the release for the target)"),
    ];
    let (f, pos) = flags::parse(&specs, args, U)?;
    let [src] = pos.as_slice() else { bail!("usage: {U}") };
    let rd = Reader::open(src).with_context(|| src.clone())?;
    let m = &rd.manifest;
    if !m.parts.is_empty() {
        bail!("{src} is a set: its parts aren't inside it. Bundle an app built with everything inside it");
    }
    if let Some(rt) = m.runtime.as_ref().filter(|r| !r.included) {
        eprintln!(
            "odi: note: {src} doesn't carry {} {}: the machine it runs on needs it. To carry it: \"include_runtime\": true in odi.json, then odi build",
            rt.name, rt.version
        );
    }
    let (os, arch) = match (f.str("platform"), m.platform.as_ref()) {
        (Some(p), _) => p
            .split_once('/')
            .map(|(o, a)| (o.to_string(), a.to_string()))
            .ok_or_else(|| anyhow!("--platform OS/ARCH"))?,
        (None, Some(p)) if !p.os.is_empty() && p.os != "any" => (p.os.clone(), p.arch.clone()),
        _ => ("linux".to_string(), crate::host_arch().to_string()),
    };
    let target = triple(&os, &arch)?;
    let runner = match f.str("runner") {
        Some(r) => PathBuf::from(r),
        None => default_runner(&os, &arch, target)?,
    };
    let exe = fs::read(&runner).with_context(|| runner.display().to_string())?;
    if !exe.windows(RUNNER_MARK.len()).any(|w| w == RUNNER_MARK.as_bytes()) {
        bail!("{} can't run a bundle (an older odi?): --runner with a newer one", runner.display());
    }
    if exe.len() as u64 >= TRAILER && &exe[exe.len() - 8..] == MAGIC {
        bail!("{} is already a bundle", runner.display());
    }
    let out = match f.str("o") {
        Some(o) => PathBuf::from(o),
        None => PathBuf::from(src.strip_suffix(".odi").unwrap_or(&format!("{src}.run"))),
    };
    let files = [PathBuf::from(src)];
    let mut lens = Vec::new();
    let tmp = out.with_extension("tmp");
    {
        let mut w = fs::File::create(&tmp).with_context(|| tmp.display().to_string())?;
        w.write_all(&exe)?;
        for f in &files {
            let mut r = fs::File::open(f).with_context(|| f.display().to_string())?;
            lens.push(std::io::copy(&mut r, &mut w)?);
        }
        for l in &lens {
            w.write_all(&l.to_le_bytes())?;
        }
        w.write_all(&(lens.len() as u64).to_le_bytes())?;
        w.write_all(MAGIC)?;
        w.sync_all()?;
    }
    let len: u64 = lens.iter().sum();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))?;
    fs::rename(&tmp, &out)?;
    println!(
        "{}  {os}/{arch}, {} ({} of runner + {} of app)",
        out.display(),
        crate::human(exe.len() as u64 + len + 8 * lens.len() as u64 + TRAILER),
        crate::human(exe.len() as u64),
        crate::human(len)
    );
    println!(
        "run it there: ./{}   (odi run's flags: --port, -e KEY=VALUE)",
        out.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
    );
    Ok(())
}

/// This odi when it's for the target (and static, on Linux), else odi's
/// release for it, of this version, checked against its SHA256SUMS and
/// kept in ~/.odi/runners.
fn default_runner(os: &str, arch: &str, target: &str) -> Result<PathBuf> {
    if os == crate::host_os() && arch == crate::host_arch() {
        let me = std::env::current_exe()?;
        if os != "linux" || is_static(&me) {
            return Ok(me);
        }
    }
    let v = env!("CARGO_PKG_VERSION");
    let dir = crate::hub::odi_home().join("runners").join(format!("v{v}")).join(target);
    let odi = dir.join("odi");
    if odi.exists() {
        return Ok(odi);
    }
    let base = format!("https://github.com/manoj1604/odi/releases/download/v{v}");
    let name = format!("odi-v{v}-{target}");
    eprintln!("odi: getting odi v{v} for {os}/{arch} to bundle with");
    let sums = get(&format!("{base}/SHA256SUMS"))
        .map_err(|e| anyhow!("{e:#}: --runner FILE (an odi built for {os}/{arch})"))?;
    let want = String::from_utf8_lossy(&sums)
        .lines()
        .find_map(|l| l.strip_suffix(&format!("  {name}.tar.gz")).map(str::to_string))
        .ok_or_else(|| anyhow!("no {name}.tar.gz in release v{v}: --runner FILE"))?;
    let tgz = get(&format!("{base}/{name}.tar.gz"))?;
    if hex::encode(Sha256::digest(&tgz)) != want.trim() {
        bail!("{name}.tar.gz doesn't match SHA256SUMS: not using it");
    }
    fs::create_dir_all(&dir)?;
    let tmp = tempfile::tempdir_in(&dir)?;
    fs::write(tmp.path().join("odi.tar.gz"), &tgz)?;
    let st = std::process::Command::new("tar").arg("-xzf").arg("odi.tar.gz").current_dir(tmp.path()).status()?;
    if !st.success() {
        bail!("couldn't unpack {name}.tar.gz");
    }
    fs::rename(tmp.path().join(&name).join("odi"), &odi)?;
    Ok(odi)
}

fn get(url: &str) -> Result<Vec<u8>> {
    let r = ureq::get(url).call().map_err(|e| anyhow!("{url}: {e}"))?;
    let mut b = Vec::new();
    r.into_reader().take(64 << 20).read_to_end(&mut b)?;
    Ok(b)
}

/// Whether an ELF executable needs no dynamic loader (no PT_INTERP).
fn is_static(p: &Path) -> bool {
    let Ok(b) = fs::read(p) else { return false };
    if b.len() < 64 || &b[..4] != b"\x7fELF" || b[4] != 2 {
        return false;
    }
    let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]) as usize;
    let u64_at = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap_or_default()) as usize;
    let (phoff, phentsize, phnum) = (u64_at(32), u16_at(54), u16_at(56));
    (0..phnum).all(|i| {
        let o = phoff + i * phentsize;
        o + 4 > b.len() || u32::from_le_bytes(b[o..o + 4].try_into().unwrap_or_default()) != 3
    })
}
