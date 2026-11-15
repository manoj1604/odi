//! Building Node.js on one platform for another (odi build --platform
//! linux/amd64 on a Mac). The project's JavaScript runs anywhere; compiled
//! packages don't. Those that come as a package per platform (sharp's
//! @img/sharp-darwin-arm64) are swapped for the target's, at the versions in
//! the lockfile. Then every compiled file in the build is read to see what it
//! really is, so one that can't be swapped stops the build, by name, instead
//! of crashing on the server.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use odi_format::Platform;
use serde::Deserialize;

use crate::build::{run, Project};
use crate::{copy_tree, exists, host_arch, host_os, mode_of, walk, Step};

#[derive(Deserialize, Default, Clone)]
pub struct NodePackage {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub os: Vec<String>,
    #[serde(default)]
    pub cpu: Vec<String>,
    #[serde(default)]
    pub libc: Vec<String>,
    #[serde(default, rename = "optionalDependencies")]
    pub optional_dependencies: BTreeMap<String, serde_json::Value>,
}

impl NodePackage {
    fn platform_specific(&self) -> bool {
        !self.os.is_empty() || !self.cpu.is_empty() || !self.libc.is_empty()
    }
}

/// A package.json list: names, or !names; empty allows everything.
pub fn allows(list: &[String], v: &str) -> bool {
    if list.is_empty() {
        return true;
    }
    let mut only = false;
    for x in list {
        if x.strip_prefix('!') == Some(v) {
            return false;
        }
        if x == v {
            return true;
        }
        only = only || !x.starts_with('!');
    }
    !only
}

struct Found {
    /// The package's folder.
    dir: PathBuf,
    pkg: NodePackage,
}

/// Every package in every node_modules under root (links aren't followed).
fn node_packages(root: &Path) -> Vec<Found> {
    let mut out = Vec::new();
    let read = |dir: PathBuf, out: &mut Vec<Found>| {
        if let Ok(b) = fs::read(dir.join("package.json")) {
            if let Ok(pkg) = serde_json::from_slice::<NodePackage>(&b) {
                if !pkg.name.is_empty() {
                    out.push(Found { dir, pkg });
                }
            }
        }
    };
    let subdirs = |d: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = fs::read_dir(d)
            .map(|r| r.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).collect())
            .unwrap_or_default();
        v.sort();
        v
    };
    let _ = walk(root, &mut |p, meta| {
        if !meta.is_dir() || p.file_name().and_then(|n| n.to_str()) != Some("node_modules") {
            return Ok(Step::Continue);
        }
        for e in subdirs(p) {
            let name = e.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('@') {
                for s in subdirs(&e) {
                    read(s, &mut out);
                }
            } else if name != ".bin" {
                read(e, &mut out);
            }
        }
        Ok(Step::Continue)
    });
    out
}

impl Project {
    /// Building for a platform other than this machine's.
    pub fn cross(&self) -> bool {
        self.goos != host_os() || self.goarch != host_arch()
    }

    /// What npm calls the target platform.
    fn npm_platform(&self) -> (&str, &str) {
        let os = if self.goos == "windows" { "win32" } else { &self.goos };
        let cpu = if self.goarch == "amd64" { "x64" } else { &self.goarch };
        (os, cpu)
    }

    pub fn fits(&self, n: &NodePackage) -> bool {
        let (os, cpu) = self.npm_platform();
        allows(&n.os, os) && allows(&n.cpu, cpu) && (os != "linux" || allows(&n.libc, "glibc"))
    }

    /// Swaps the platform packages in the build for the target's.
    pub fn cross_node(&self) -> Result<()> {
        let all = node_packages(&self.stage);
        let foreign: Vec<&Found> = all.iter().filter(|n| n.pkg.platform_specific() && !self.fits(&n.pkg)).collect();
        if foreign.is_empty() {
            return Ok(());
        }
        let target = self.install_for_target()?;
        let mut available: HashMap<String, Found> = HashMap::new();
        for n in node_packages(target.path()) {
            if n.pkg.platform_specific() && self.fits(&n.pkg) {
                available.insert(n.pkg.name.clone(), n);
            }
        }
        for f in foreign {
            // Its family: what the package asking for it offers for other platforms.
            let mut family: Vec<&String> = all
                .iter()
                .filter(|n| n.pkg.optional_dependencies.contains_key(&f.pkg.name))
                .flat_map(|n| n.pkg.optional_dependencies.keys())
                .collect();
            family.sort();
            fs::remove_dir_all(&f.dir)?;
            // (node_modules, or node_modules/@scope for a scoped name)
            let mut base = f.dir.clone();
            for _ in f.pkg.name.split('/') {
                base.pop();
            }
            let mut matched = false;
            for name in family {
                let Some(a) = available.get(name) else { continue };
                matched = true;
                let dst = base.join(name);
                if exists(&dst, "package.json") {
                    continue;
                }
                copy_tree(&a.dir, &dst)?;
                eprintln!("== {} -> {}", f.pkg.name, name);
            }
            if !matched {
                bail!(
                    "{} has no build for {}/{}: build this project on {}",
                    f.pkg.name,
                    self.goos,
                    self.goarch,
                    self.goos
                );
            }
        }
        Ok(())
    }

    /// The project's dependencies, from its lockfile, as npm would install
    /// them on the target, in a folder of their own. Nothing runs: install
    /// scripts would run on this machine.
    fn install_for_target(&self) -> Result<tempfile::TempDir> {
        let dir = tempfile::Builder::new().prefix("odi-target-").tempdir()?;
        for f in ["package.json", "package-lock.json", ".npmrc"] {
            if let Ok(b) = fs::read(self.dir.join(f)) {
                fs::write(dir.path().join(f), b)?;
            }
        }
        let (os, cpu) = self.npm_platform();
        let verb = if exists(dir.path(), "package-lock.json") { "ci" } else { "install" };
        let (os_arg, cpu_arg) = (format!("--os={os}"), format!("--cpu={cpu}"));
        let mut args = vec![verb, "--omit=dev", "--ignore-scripts", "--no-audit", "--no-fund", &os_arg, &cpu_arg];
        if os == "linux" {
            args.push("--libc=glibc");
        }
        eprintln!("== fetching packages for {}/{}", self.goos, self.goarch);
        run(dir.path(), &[], "npm", &args)?;
        Ok(dir)
    }

    /// The compiled files in the build, and those not for the target.
    pub fn native_files(&self) -> Result<(usize, Vec<String>)> {
        let mut native = 0;
        let mut wrong = Vec::new();
        let mut seen = HashSet::new();
        walk(&self.stage, &mut |p, meta| {
            if !meta.file_type().is_file() {
                return Ok(Step::Continue);
            }
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let lib =
                name.ends_with(".node") || name.ends_with(".dylib") || name.ends_with(".so") || name.contains(".so.");
            if !lib && mode_of(meta) & 0o111 == 0 {
                return Ok(Step::Continue);
            }
            let Some((os, arches)) = binary_platform(p) else { return Ok(Step::Continue) };
            native += 1;
            let ok = os == self.goos && arches.contains(&self.goarch);
            let pkg = package_of(&self.stage, p);
            if !ok && seen.insert(pkg.clone()) {
                wrong.push(format!("{pkg} (built for {os}/{})", arches.join(",")));
            }
            Ok(Step::Continue)
        })?;
        Ok((native, wrong))
    }

    /// What's compiled in the build ties it to the target; nothing compiled,
    /// and it runs anywhere its runtime does.
    pub fn platform_from_files(&mut self) -> Result<()> {
        let (native, wrong) = self.native_files()?;
        if native == 0 {
            self.set_platform("any", "any");
            return Ok(());
        }
        if self.cross() && !wrong.is_empty() {
            bail!(
                "can't make this build for {}/{}: {}. These compile when installed, with no ready-made {} build; build this project on {}",
                self.goos,
                self.goarch,
                wrong.join(", "),
                self.goos,
                self.goos
            );
        }
        let (os, arch) = (self.goos.clone(), self.goarch.clone());
        self.set_platform(&os, &arch);
        Ok(())
    }

    fn set_platform(&mut self, os: &str, arch: &str) {
        let libc = if os == "linux" { "gnu" } else { "" };
        self.m.platform = Some(Platform { os: os.into(), arch: arch.into(), libc: libc.into() });
    }
}

/// The package a path in the build belongs to, for messages.
pub fn package_of(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
    let parts: Vec<&str> = rel.split('/').collect();
    for i in (0..parts.len().saturating_sub(1)).rev() {
        if parts[i] == "node_modules" && i + 1 < parts.len() {
            if parts[i + 1].starts_with('@') && i + 2 < parts.len() {
                return format!("{}/{}", parts[i + 1], parts[i + 2]);
            }
            return parts[i + 1].to_string();
        }
    }
    rel
}

fn elf_arch(machine: u16) -> String {
    match machine {
        62 => "amd64".into(),
        183 => "arm64".into(),
        3 => "386".into(),
        40 => "arm".into(),
        243 => "riscv64".into(),
        m => format!("EM_{m}"),
    }
}

fn macho_arch(cpu: u32) -> String {
    match cpu {
        0x0100_0007 => "amd64".into(),
        0x0100_000c => "arm64".into(),
        7 => "386".into(),
        12 => "arm".into(),
        c => format!("cpu{c:#x}"),
    }
}

/// Reads what a compiled file is for (ELF, Mach-O, or a universal Mach-O):
/// None when it isn't one.
pub fn binary_platform(path: &Path) -> Option<(String, Vec<String>)> {
    let mut f = fs::File::open(path).ok()?;
    let mut h = [0u8; 64];
    let n = f.read(&mut h).ok()?;
    let h = &h[..n];
    let u16le = |i: usize| h.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u16be = |i: usize| h.get(i..i + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let u32at = |b: &[u8], i: usize, le: bool| {
        b.get(i..i + 4).map(|b| {
            let a = [b[0], b[1], b[2], b[3]];
            if le {
                u32::from_le_bytes(a)
            } else {
                u32::from_be_bytes(a)
            }
        })
    };
    if h.starts_with(b"\x7fELF") {
        // class (1, 2) and data (1: little-endian, 2: big) at 4 and 5.
        if !matches!(h.get(4), Some(1 | 2)) {
            return None;
        }
        let machine = match h.get(5) {
            Some(1) => u16le(18)?,
            Some(2) => u16be(18)?,
            _ => return None,
        };
        return Some(("linux".into(), vec![elf_arch(machine)]));
    }
    match h.get(..4)? {
        [0xcf, 0xfa, 0xed, 0xfe] | [0xce, 0xfa, 0xed, 0xfe] => {
            Some(("darwin".into(), vec![macho_arch(u32at(h, 4, true)?)]))
        }
        [0xfe, 0xed, 0xfa, 0xcf] | [0xfe, 0xed, 0xfa, 0xce] => {
            Some(("darwin".into(), vec![macho_arch(u32at(h, 4, false)?)]))
        }
        [0xca, 0xfe, 0xba, 0xbe] => {
            // A universal file (or a Java class, which starts the same): a few
            // architectures, each a Mach-O file inside it.
            let narch = u32at(h, 4, false)? as usize;
            if !(1..=20).contains(&narch) {
                return None;
            }
            let data = fs::read(path).ok()?;
            let mut arches = Vec::new();
            for i in 0..narch {
                let at = 8 + i * 20;
                let cpu = u32at(&data, at, false)?;
                let off = u32at(&data, at + 8, false)? as usize;
                let magic = data.get(off..off + 4)?;
                if !matches!(magic, [0xcf, 0xfa, 0xed, 0xfe] | [0xce, 0xfa, 0xed, 0xfe]) {
                    return None;
                }
                arches.push(macho_arch(cpu));
            }
            Some(("darwin".into(), arches))
        }
        _ => None,
    }
}
