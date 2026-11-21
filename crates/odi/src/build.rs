//! odi build: a project to an ODI file, with the project's own tools. What it
//! knows (the first that fits):
//!
//! - Go: go.mod: a static executable (CGO_ENABLED=0) for --platform
//! - Next.js: package.json with next: npm run build, .next/standalone in
//!   app/ with .next/static and public/ (--static odi: also in static/,
//!   served by odi)
//! - static: package.json whose build makes dist/ (or build/, out/) with
//!   index.html and no start script; or index.html at the top
//! - Node.js: package.json: the project and its production dependencies
//!   (npm ci --omit=dev), started by its start script or main
//! - Python: requirements.txt, pyproject.toml or a main file: the project
//!   and its packages (pip install --target app/.packages)
//!
//! A Procfile's web line, and odi.json in the project (src/odijson.rs: the
//! runtime, whether to include it, the build and start commands, the port,
//! environment variables, and any manifest field), override what was found. The build time is the last commit's (or
//! SOURCE_DATE_EPOCH), so the same commit builds to the same digest.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};
use odi_format::{digest_string, Build, Env, Language, Manifest, Platform, Port, Process, Runtime, Static};
use serde::Deserialize;
use serde_json::Value;

use crate::flags::{self, flag, Kind::*};
use crate::odijson::{self, Kind, OdiJson};
use crate::{copy_one, copy_tree, exists, host_arch, host_os, pack, walk, Step, VERSION};

pub struct Project {
    pub dir: PathBuf,
    pub stage: PathBuf,
    pub goos: String,
    pub goarch: String,
    pub build: bool,
    /// Next.js: odi serves .next/static and public/ (--static odi), not the
    /// app.
    pub odi_static: bool,
    /// The project's odi.json.
    pub file: OdiJson,
    pub m: Manifest,
}

pub fn cmd_build(args: &[String]) -> Result<()> {
    const U: &str =
        "odi build [PROJECT] [-o FILE] [--platform OS/ARCH] [--no-build] [--runtime PART] [--static app|odi] [--compress max] [--profile [--path /]... [-e KEY=VALUE]...]";
    let specs = [
        flag("o", Value, "the ODI file (default: NAME.odi)"),
        flag("platform", Value, "OS/ARCH to build for (Go, Node.js, Next.js; Python: this machine's)"),
        flag("no-build", Bool, "don't run the project's build script"),
        flag("profile", Bool, "then run it and keep what it reads while starting (Linux; see odi profile)"),
        flag("path", Multi, "with --profile: a URL path to ask for (repeat; default /)"),
        flag("e", Multi, "with --profile: KEY=VALUE for the program (repeat)"),
        flag("port", Value, "with --profile: the port to run it on"),
        flag("use-path-runtime", Bool, "with --profile: run a runtime pinned by digest with this machine's"),
        flag("runtime", Value, "a runtime part (file or registry name) to pin, by digest and exact version"),
        flag("compress", Value, "default, or max (zstd 19: about 10% smaller, as fast to read)"),
        flag(
            "static",
            Value,
            "Next.js: who serves .next/static and public/: app (default) or odi (cached forever, CDN-ready)",
        ),
    ];
    let (f, pos) = flags::parse(&specs, args, U)?;
    if pos.len() > 1 {
        bail!("usage: {U}");
    }
    crate::set_compress(f.str("compress"))?;
    let dir = std::path::absolute(pos.first().map(String::as_str).unwrap_or("."))?;
    let plat = f.str("platform").map(String::from).unwrap_or_else(|| format!("{}/{}", host_os(), host_arch()));
    let Some((goos, goarch)) = plat.split_once('/') else { bail!("--platform: OS/ARCH, e.g. linux/amd64") };
    let odi_static = match f.str("static") {
        None | Some("app") => false,
        Some("odi") => true,
        Some(o) => bail!("--static {o}: app or odi"),
    };
    let stage = tempfile::Builder::new().prefix("odi-build-").tempdir()?;
    let base = dir.file_name().and_then(|n| n.to_str()).unwrap_or("app");
    let mut p = Project {
        dir: dir.clone(),
        stage: stage.path().to_path_buf(),
        goos: goos.into(),
        goarch: goarch.into(),
        build: !f.bool("no-build"),
        odi_static,
        file: OdiJson::load(&dir)?,
        m: Manifest { odi: "0.1".into(), name: safe_name(base), ..Default::default() },
    };

    let t = p.file.kind.as_str();
    let need = |files: &[&str]| -> Result<()> {
        if files.iter().any(|f| exists(&dir, f)) {
            Ok(())
        } else {
            bail!("odi.json says \"type\": \"{t}\", but there's no {} here", files.join(" or "))
        }
    };
    let kind = match p.file.kind {
        Kind::Go => {
            need(&["go.mod"])?;
            p.pre_build()?;
            p.build_go()?;
            "Go".to_string()
        }
        Kind::Node | Kind::Nextjs => {
            need(&["package.json"])?;
            p.build_node()?
        }
        Kind::Python => {
            need(&["requirements.txt", "pyproject.toml", "app.py", "main.py"])?;
            p.pre_build()?;
            p.build_python()?;
            "Python".into()
        }
        Kind::Static => p.build_static_project()?,
    };
    p.procfile()?;
    p.overrides()?;
    p.build_info();
    if p.file.include_runtime {
        if f.str("runtime").is_some() {
            bail!("odi.json's include_runtime and --runtime both say where the runtime comes from: one of them");
        }
        p.include_runtime()?;
    }
    if let Some(r) = f.str("runtime") {
        p.pin_runtime(r)?;
    }
    p.route_map()?;
    if exists(&dir, "README.md") {
        // (what a registry shows)
        fs::create_dir_all(p.stage.join("meta"))?;
        copy_tree(&dir.join("README.md"), &p.stage.join("meta").join("README.md"))?;
    }
    let out = f.str("o").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("{}.odi", p.m.name)));
    eprintln!("== {kind}: packing");
    let d = pack(&p.stage, &mut p.m, &out)?;
    println!("{} {}", out.display(), digest_string(&d));
    if f.bool("profile") {
        if host_os() != "linux" || p.cross() {
            bail!("--profile runs the build, mounted: on Linux, for this machine (here: build without it, then odi profile on Linux)");
        }
        eprintln!("== profiling");
        crate::run::profile(&out, &out, f.all("path"), &crate::run::RunOpts::from_flags(&f)?)?;
    }
    Ok(())
}

/// odi init: an odi.json for the project here, from what odi finds in it
/// (docs/odi-json.md).
pub fn cmd_init(args: &[String]) -> Result<()> {
    const U: &str = "odi init [PROJECT] [--type go|node|nextjs|python|static] [--name NAME]";
    let specs = [
        flag("type", Value, "what kind of project, when odi can't tell"),
        flag("name", Value, "the app's name (default: package.json's, go.mod's, or the folder's)"),
    ];
    let (f, pos) = flags::parse(&specs, args, U)?;
    if pos.len() > 1 {
        bail!("usage: {U}");
    }
    let dir = std::path::absolute(pos.first().map(String::as_str).unwrap_or("."))?;
    let path = dir.join("odi.json");
    if path.exists() {
        bail!("{} has one already: edit it (docs/odi-json.md)", path.display());
    }
    let pkg: Option<PackageJson> = fs::read(dir.join("package.json"))
        .ok()
        .map(|b| serde_json::from_slice(&b))
        .transpose()
        .context("package.json")?;
    let kind = match f.str("type") {
        Some(t) => Kind::parse(t).ok_or_else(|| anyhow!("--type {t}: go, node, nextjs, python or static"))?,
        None if exists(&dir, "go.mod") => Kind::Go,
        None if pkg.as_ref().is_some_and(|p| p.dependencies.contains_key("next")) => Kind::Nextjs,
        None if pkg.as_ref().is_some_and(|p| {
            !p.scripts.contains_key("start") && p.main.is_empty() && p.scripts.contains_key("build")
        }) =>
        {
            Kind::Static
        }
        None if pkg.is_some() => Kind::Node,
        None if ["requirements.txt", "pyproject.toml", "app.py", "main.py"].iter().any(|f| exists(&dir, f)) => {
            Kind::Python
        }
        None if exists(&dir, "index.html") => Kind::Static,
        None => bail!("odi can't tell what kind of project this is: odi init --type go|node|nextjs|python|static"),
    };
    let base = dir.file_name().and_then(|n| n.to_str()).unwrap_or("app");
    let name = match f.str("name") {
        Some(n) => n.to_string(),
        None => match &pkg {
            Some(p) if !p.name.is_empty() => safe_name(&p.name),
            _ if kind == Kind::Go => {
                let m = output(&dir, "go", &["list", "-m"]);
                safe_name(m.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(base))
            }
            _ => safe_name(base),
        },
    };
    let mut lines = vec![
        format!("  \"$schema\": {}", serde_json::to_string(odijson::SCHEMA)?),
        format!("  \"name\": {}", serde_json::to_string(&name)?),
        format!("  \"type\": \"{}\"", kind.as_str()),
    ];
    if kind.runtime() == Some("node") {
        let p = Project {
            dir: dir.clone(),
            stage: dir.clone(),
            goos: host_os().into(),
            goarch: host_arch().into(),
            build: false,
            odi_static: false,
            file: OdiJson::new(&name, kind),
            m: Manifest::default(),
        };
        let v = p.node_version(pkg.as_ref().expect("a node project has package.json"));
        let major: String =
            v.trim_start_matches(['^', '~', '>', '=', 'v', ' ']).chars().take_while(|c| c.is_ascii_digit()).collect();
        if !major.is_empty() {
            lines.push(format!("  \"runtime\": \"node {major}\""));
        }
    }
    let text = format!("{{\n{}\n}}\n", lines.join(",\n"));
    OdiJson::parse(text.as_bytes()).context("the odi.json odi init made")?;
    fs::write(&path, &text)?;
    print!("{}:\n{text}", path.display());
    if kind == Kind::Nextjs {
        eprintln!("(Next.js: set output: \"standalone\" in next.config, which odi builds from.)");
    }
    eprintln!("Then: odi build. Every key: https://github.com/manoj1604/odi/blob/main/docs/odi-json.md");
    Ok(())
}

/// Runs a command in dir, its output to stderr.
pub fn run(dir: &Path, env: &[(&str, &str)], name: &str, args: &[&str]) -> Result<()> {
    eprintln!("== {} {}", name, args.join(" "));
    let st = Command::new(name)
        .args(args)
        .current_dir(dir)
        .envs(env.iter().copied())
        .stdout(std::io::stderr())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| name.to_string())?;
    if !st.success() {
        bail!("{} {}: {st}", name, args.join(" "));
    }
    Ok(())
}

/// A command's output, trimmed; "" if it fails.
pub fn output(dir: &Path, name: &str, args: &[&str]) -> String {
    Command::new(name)
        .args(args)
        .current_dir(dir)
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// A name the manifest takes: runs of other characters become `-`.
pub fn safe_name(s: &str) -> String {
    let s = s.strip_prefix('@').unwrap_or(s);
    let mut out = String::new();
    let mut dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
            out.push(c);
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    let mut s = out.trim_matches(|c| c == '-' || c == '.').to_string();
    if s.is_empty() {
        s = "app".into();
    }
    s.truncate(128);
    s
}

/// Copies the project into dst, without what never ships.
fn copy_project(src: &Path, dst: &Path, skip: &[&str]) -> Result<()> {
    let mut never: HashSet<&str> =
        [".git", "node_modules", ".venv", "venv", "__pycache__", ".DS_Store", ".next", ".turbo", ".cache"].into();
    never.extend(skip);
    walk(src, &mut |p, meta| {
        if p == src {
            fs::create_dir_all(dst)?;
            return Ok(Step::Continue);
        }
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if never.contains(name) || name.ends_with(".odi") {
            return Ok(Step::SkipDir);
        }
        copy_one(p, &dst.join(p.strip_prefix(src)?), meta)?;
        Ok(Step::Continue)
    })
}

fn read_file(p: &Path) -> String {
    fs::read_to_string(p).unwrap_or_default()
}

impl Project {
    // --- Go ---------------------------------------------------------------------

    fn build_go(&mut self) -> Result<()> {
        let module = output(&self.dir, "go", &["list", "-m"]);
        if !module.is_empty() {
            self.m.name = safe_name(module.rsplit('/').next().unwrap_or(&module));
        }
        let app = self.stage.join("app");
        fs::create_dir_all(&app)?;
        let out = app.join(&self.m.name);
        let out = out.to_str().ok_or_else(|| anyhow!("a temporary folder that isn't UTF-8"))?;
        let env = [("CGO_ENABLED", "0"), ("GOOS", self.goos.as_str()), ("GOARCH", self.goarch.as_str())];
        run(&self.dir, &env, "go", &["build", "-trimpath", "-ldflags", "-s -w", "-o", out, "."])?;
        let libc = if self.goos == "linux" { "none" } else { "" };
        self.m.platform = Some(Platform { os: self.goos.clone(), arch: self.goarch.clone(), libc: libc.into() });
        self.m.language = Some(Language { name: "go".into(), framework: String::new() });
        self.m.processes = vec![web(vec![self.m.name.clone()], 8080)];
        Ok(())
    }

    /// Pins the runtime to a runtime part: its exact version and digest. A
    /// runner then provides that one (from its store or a registry), not any
    /// it finds of the same line.
    fn pin_runtime(&mut self, src: &str) -> Result<()> {
        let s = crate::parts::source(src)?;
        let pm = &s.rd.manifest;
        let Some(gives) = pm.provides.iter().find(|g| g.plug == "runtime") else {
            bail!("--runtime {src}: not a runtime part (it provides none)");
        };
        if let Some(pp) = &pm.platform {
            if (pp.os != self.goos && pp.os != "any") || (pp.arch != self.goarch && pp.arch != "any") {
                bail!("--runtime {src}: it's for {}/{}, the build for {}/{}", pp.os, pp.arch, self.goos, self.goarch);
            }
        }
        let Some(rt) = self.m.runtime.as_mut() else { bail!("--runtime: this build needs no runtime") };
        if rt.name != gives.name {
            bail!("--runtime {src}: it's {}, this build runs on {}", gives.name, rt.name);
        }
        if !rt.version.is_empty() && !crate::run::version_matches(&gives.version, &rt.version) {
            bail!("--runtime {src}: {} {}, but the project asks for {}", gives.name, gives.version, rt.version);
        }
        rt.version = gives.version.clone();
        rt.digest = digest_string(&s.rd.digest);
        rt.included = false;
        eprintln!("== runtime: {} {} pinned ({})", rt.name, rt.version, rt.digest);
        Ok(())
    }

    // --- Node.js ----------------------------------------------------------------

    fn build_node(&mut self) -> Result<String> {
        let b = fs::read(self.dir.join("package.json")).unwrap_or_default();
        let pkg: PackageJson = serde_json::from_slice(&b).context("package.json")?;
        if !pkg.name.is_empty() {
            self.m.name = safe_name(&pkg.name);
        }
        self.m.language = Some(Language { name: "node".into(), framework: String::new() });
        self.m.runtime = Some(Runtime { name: "node".into(), version: self.node_version(&pkg), ..Default::default() });
        let next = pkg.dependencies.contains_key("next");
        match self.file.kind {
            Kind::Node if next => bail!("odi.json: this project depends on next: \"type\": \"nextjs\""),
            Kind::Nextjs if !next => {
                bail!("odi.json says \"type\": \"nextjs\", but package.json doesn't depend on next")
            }
            _ => {}
        }
        self.npm_build(&pkg)?;
        let script = |s: &str| pkg.scripts.get(s).map(String::as_str).unwrap_or("");
        if next {
            self.nextjs()?;
            return Ok("Next.js".into());
        }
        let app = self.stage.join("app");
        copy_project(&self.dir, &app, &[])?;
        if !pkg.dependencies.is_empty() {
            npm_install(&app, true)?;
        }
        let start = script("start");
        let fields: Vec<String> = start.split_whitespace().map(String::from).collect();
        let cmd = if fields.len() >= 2 && fields[0] == "node" && !start.contains(['&', '|', ';', '$']) {
            fields
        } else if start.is_empty() && !pkg.main.is_empty() {
            vec!["node".into(), pkg.main.clone()]
        } else if start.is_empty() {
            vec!["node".into(), "index.js".into()]
        } else {
            vec!["npm".into(), "start".into()]
        };
        self.m.processes = vec![web(cmd, 3000)];
        self.m.env = Some(defaults(&[("NODE_ENV", "production")]));
        self.node_platform()?;
        Ok("Node.js".into())
    }

    /// odi.json's build command, for a project odi builds without npm.
    fn pre_build(&self) -> Result<()> {
        match self.file.build.as_deref() {
            Some(cmd) if self.build && !cmd.is_empty() => run(&self.dir, &[], "sh", &["-c", cmd]),
            _ => Ok(()),
        }
    }

    /// npm ci (when there's no node_modules) and the build: odi.json's
    /// command, else the build script ("" builds nothing).
    fn npm_build(&self, pkg: &PackageJson) -> Result<()> {
        if !self.build {
            return Ok(());
        }
        let cmd = match self.file.build.as_deref() {
            Some("") => return Ok(()),
            Some(c) => c.to_string(),
            None if pkg.scripts.contains_key("build") => "npm run build".into(),
            None => return Ok(()),
        };
        if !exists(&self.dir, "node_modules") && !pkg.dependencies.is_empty() {
            npm_install(&self.dir, false)?;
        }
        if cmd == "npm run build" {
            run(&self.dir, &[], "npm", &["run", "build"])
        } else {
            run(&self.dir, &[], "sh", &["-c", &cmd])
        }
    }

    /// A static project: built if it says how (or has a package.json build
    /// script), then its folder served.
    fn build_static_project(&mut self) -> Result<String> {
        if exists(&self.dir, "package.json") {
            let b = fs::read(self.dir.join("package.json")).unwrap_or_default();
            let pkg: PackageJson = serde_json::from_slice(&b).context("package.json")?;
            self.npm_build(&pkg)?;
        } else {
            self.pre_build()?;
        }
        let folder = match &self.file.r#static {
            Some(f) => {
                let d = self.dir.join(f);
                if !d.is_dir() {
                    bail!("odi.json: \"static\": {f}: no such folder (after the build)");
                }
                d
            }
            None => ["dist", "build", "out"]
                .iter()
                .map(|f| self.dir.join(f))
                .find(|d| exists(d, "index.html"))
                .unwrap_or_else(|| self.dir.clone()),
        };
        self.build_static(&folder)?;
        let shown = folder.strip_prefix(&self.dir).ok().map(|p| p.display().to_string()).filter(|p| !p.is_empty());
        Ok(match shown {
            Some(p) => format!("static site ({p}/)"),
            None => "static site".into(),
        })
    }

    fn node_version(&self, pkg: &PackageJson) -> String {
        if let Some((n, v)) = &self.file.runtime {
            if n == "node" && !v.is_empty() {
                return v.clone();
            }
        }
        for f in [".nvmrc", ".node-version"] {
            let v = read_file(&self.dir.join(f));
            let v = v.trim();
            let v = v.strip_prefix('v').unwrap_or(v);
            if !v.is_empty() {
                return v.into();
            }
        }
        if !pkg.engines.node.is_empty() {
            return pkg.engines.node.clone();
        }
        // (This machine's Node, as its major line: 22, not 22.23.3, so any 22 runs it.)
        let v = output(&self.dir, "node", &["--version"]);
        let v = v.strip_prefix('v').unwrap_or(&v);
        v.split('.').next().unwrap_or("").into()
    }

    fn nextjs(&mut self) -> Result<()> {
        let sa = self.dir.join(".next").join("standalone");
        if !exists(&sa, "server.js") {
            bail!(
                r#"Next.js: .next/standalone/server.js is missing: set output: "standalone" in next.config, then build"#
            );
        }
        copy_tree(&sa, &self.stage.join("app"))?;
        // .next/static and public/ in the app, where Next.js serves them from:
        // `node server.js` serves the whole site, in odi run or by hand. With
        // --static odi, also in static/, served by odi run itself (cached
        // forever; a host can put them on a CDN). Identical files are kept
        // once in an ODI file, so the second copy costs next to nothing.
        if exists(&self.dir, ".next/static") {
            copy_tree(&self.dir.join(".next/static"), &self.stage.join("app/.next/static"))?;
            if self.odi_static {
                copy_tree(&self.dir.join(".next/static"), &self.stage.join("static/_next/static"))?;
                self.m.r#static.push(Static {
                    from: "static/_next/static".into(),
                    at: "/_next/static".into(),
                    immutable: true,
                });
            }
        }
        if exists(&self.dir, "public") {
            copy_tree(&self.dir.join("public"), &self.stage.join("app/public"))?;
            if self.odi_static {
                copy_tree(&self.dir.join("public"), &self.stage.join("static/public"))?;
                self.m.r#static.push(Static { from: "static/public".into(), at: "/".into(), immutable: false });
            }
        }
        if let Some(l) = &mut self.m.language {
            l.framework = "nextjs".into();
        }
        self.m.processes = vec![web(vec!["node".into(), "server.js".into()], 3000)];
        self.m.env = Some(defaults(&[("NODE_ENV", "production"), ("HOSTNAME", "0.0.0.0")]));
        self.m.writable = vec!["app/.next/cache".into()]; // (resized images)
        self.node_platform()
    }

    /// The target's packages in the build, and the platform it's for.
    fn node_platform(&mut self) -> Result<()> {
        if self.cross() {
            self.cross_node()?;
        }
        self.platform_from_files()
    }

    // --- Python -----------------------------------------------------------------

    fn build_python(&mut self) -> Result<()> {
        if self.cross() {
            bail!(
                "Python builds are for this machine for now, not {}/{}: build on {}",
                self.goos,
                self.goarch,
                self.goos
            );
        }
        let app = self.stage.join("app");
        copy_project(&self.dir, &app, &[])?;
        let pip = ["-m", "pip", "install", "--quiet", "--disable-pip-version-check", "--target", ".packages"];
        if exists(&self.dir, "requirements.txt") {
            run(&app, &[], "python3", &[&pip[..], &["-r", "requirements.txt"]].concat())?;
        } else if exists(&self.dir, "pyproject.toml") {
            run(&app, &[], "python3", &[&pip[..], &["."]].concat())?;
        }
        let main = ["app.py", "main.py", "server.py"].into_iter().find(|f| exists(&self.dir, f)).unwrap_or("app.py");
        let mut v = read_file(&self.dir.join(".python-version")).trim().to_string();
        if v.is_empty() {
            let out = output(&self.dir, "python3", &["--version"]);
            v = out.strip_prefix("Python ").unwrap_or(&out).to_string();
            if let Some(i) = v.rfind('.') {
                v.truncate(i); // (3.13.5 -> 3.13: any patch release)
            }
        }
        self.m.language = Some(Language { name: "python".into(), framework: String::new() });
        self.m.runtime = Some(Runtime { name: "python".into(), version: v, ..Default::default() });
        self.m.processes = vec![web(vec!["python3".into(), main.into()], 8000)];
        self.m.env = Some(defaults(&[("PYTHONPATH", ".packages"), ("PYTHONUNBUFFERED", "1")]));
        self.platform_from_files()
    }

    // --- static sites -----------------------------------------------------------

    fn build_static(&mut self, dir: &Path) -> Result<()> {
        copy_project(dir, &self.stage.join("static"), &["package.json", "package-lock.json", "odi.json", "Procfile"])?;
        if self.m.language.is_none() {
            self.m.language = Some(Language { name: "static".into(), framework: String::new() });
        }
        self.m.r#static = vec![Static { from: "static".into(), at: "/".into(), immutable: false }];
        self.m.platform = Some(Platform { os: "any".into(), arch: "any".into(), libc: String::new() });
        Ok(())
    }

    // --- common -----------------------------------------------------------------

    /// Each route's files, and each static page's assets, for a runner that
    /// fetches as it reads (a hint: meta/odi/routes.json).
    fn route_map(&self) -> Result<()> {
        let mut routes = match crate::hints::next_routes(&self.stage)? {
            Some(map) => map.routes,
            None => Vec::new(),
        };
        let pages = crate::hints::static_pages(&self.stage, &self.m.r#static);
        let n = (routes.len(), pages.len());
        routes.extend(pages);
        if routes.is_empty() {
            return Ok(());
        }
        let dir = self.stage.join("meta/odi");
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("routes.json"), serde_json::to_vec(&crate::hints::RouteMap { version: 1, routes })?)?;
        eprintln!("== route map: {} routes, {} static pages", n.0, n.1);
        Ok(())
    }

    /// A Procfile's processes (web, worker...) replace the ones found.
    fn procfile(&mut self) -> Result<()> {
        let Ok(text) = fs::read_to_string(self.dir.join("Procfile")) else { return Ok(()) };
        let mut procs = Vec::new();
        for line in text.lines() {
            let Some((name, cmd)) = line.split_once(':') else { continue };
            let (name, cmd) = (name.trim(), cmd.trim());
            if name.is_empty() || name.starts_with('#') || cmd.is_empty() {
                continue;
            }
            let mut pr = Process {
                name: name.into(),
                command: vec!["sh".into(), "-c".into(), cmd.into()],
                ..Default::default()
            };
            if name == "web" {
                pr.port = Some(Port::http(3000));
                if let Some(old) = self.m.processes.iter().find(|o| o.name == "web" && o.port.is_some()) {
                    pr.port = old.port.clone();
                }
            }
            procs.push(pr);
        }
        if !procs.is_empty() {
            self.m.processes = procs;
        }
        Ok(())
    }

    /// odi.json in the project, any manifest fields, over what was found
    /// (objects merged, the rest replaced; null removes).
    fn overrides(&mut self) -> Result<()> {
        // Manifest fields, merged in as they are ("repo" is where odi push
        // publishes, not part of the manifest).
        self.m.name = self.file.name.clone();
        if let Some(v) = &self.file.version {
            self.m.version = v.clone();
        }
        let mut cur = serde_json::to_value(&self.m)?;
        merge(&mut cur, Value::Object(self.file.manifest.clone()));
        self.m = serde_json::from_value(cur).context("odi.json's manifest")?;

        // Then the plain keys.
        if let Some((name, version)) = self.file.runtime.clone() {
            let rt = self.m.runtime.get_or_insert_with(Runtime::default);
            if !rt.name.is_empty() && rt.name != name {
                bail!("odi.json: \"runtime\": {name}, but this is a {} project", rt.name);
            }
            rt.name = name;
            if !version.is_empty() {
                rt.version = version;
            }
        }
        if let Some(cmd) = self.file.start.clone() {
            let command = odijson::command(&cmd);
            match self.m.processes.iter_mut().find(|p| p.name == "web") {
                Some(w) => w.command = command,
                None => self.m.processes.insert(0, web(command, 3000)),
            }
        }
        if let Some(port) = self.file.port {
            let Some(w) = self.m.processes.iter_mut().find(|p| p.name == "web") else {
                bail!("odi.json: \"port\", but nothing to start: add \"start\"");
            };
            w.port = Some(Port::http(port));
        }
        if !self.file.env.is_empty() {
            let env = self.m.env.get_or_insert_with(Env::default);
            for (k, v) in &self.file.env {
                env.required.retain(|x| x != k);
                env.optional.retain(|x| x != k);
                env.defaults.remove(k);
                match v.as_str() {
                    "required" => env.required.push(k.clone()),
                    "optional" => env.optional.push(k.clone()),
                    _ => {
                        env.defaults.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        Ok(())
    }

    /// odi.json's include_runtime: the runtime in the file itself (runtime/),
    /// the official release for the target, checked against its published
    /// checksums. Node.js for now.
    fn include_runtime(&mut self) -> Result<()> {
        let Some(rt) = self.m.runtime.clone().filter(|r| r.name == "node") else {
            bail!("odi.json: include_runtime: odi can put Node.js in a file for now");
        };
        let v = crate::parts::node_exact(&rt.version)?;
        let plat = format!("{}/{}", self.goos, self.goarch);
        eprintln!("== runtime: node {v} for {plat}, in the file");
        crate::parts::node_release(&v, &plat, &self.stage.join("runtime"))?;
        let r = self.m.runtime.as_mut().expect("checked");
        r.version = v;
        r.included = true;
        r.digest.clear();
        // (nodejs.org's Linux builds are for glibc.)
        let libc = if self.goos == "linux" { "gnu" } else { "" };
        self.m.platform = Some(Platform { os: self.goos.clone(), arch: self.goarch.clone(), libc: libc.into() });
        Ok(())
    }

    /// Where it came from (no credentials), and when: the last commit's time,
    /// so a commit always builds to the same digest.
    fn build_info(&mut self) {
        let mut bi = Build { builder: format!("odi {VERSION}"), ..Default::default() };
        let src = output(&self.dir, "git", &["remote", "get-url", "origin"]);
        if !src.is_empty() {
            bi.source = match url::Url::parse(&src) {
                Ok(mut u) if !u.username().is_empty() || u.password().is_some() => {
                    let _ = u.set_username("");
                    let _ = u.set_password(None);
                    u.to_string()
                }
                _ => src,
            };
        }
        bi.revision = output(&self.dir, "git", &["rev-parse", "HEAD"]);
        let secs = match std::env::var("SOURCE_DATE_EPOCH") {
            Ok(s) => s.trim().parse::<i64>().ok(),
            Err(_) => output(&self.dir, "git", &["log", "-1", "--format=%ct"]).parse::<i64>().ok(),
        };
        if let Some(t) = secs.and_then(|s| chrono::DateTime::from_timestamp(s, 0)) {
            bi.time = t.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        }
        // The version: odi.json's; else a version tag on this commit
        // (v1.4.2), the project's own (package.json, pyproject.toml), or the
        // commit.
        if self.m.version.is_empty() {
            self.m.version = self.project_version();
        }
        if !bi.revision.is_empty() && self.m.version.is_empty() {
            self.m.version = bi.revision.chars().take(12).collect();
        }
        self.m.build = Some(bi);
    }
}

impl Project {
    fn project_version(&self) -> String {
        let tags = output(&self.dir, "git", &["tag", "--points-at", "HEAD"]);
        let mut found: Vec<crate::versions::Version> =
            tags.lines().filter_map(|t| crate::versions::Version::parse(t.trim())).collect();
        found.sort();
        if let Some(v) = found.pop() {
            return v.to_string().trim_start_matches('v').to_string();
        }
        if let Ok(b) = fs::read(self.dir.join("package.json")) {
            if let Some(v) =
                serde_json::from_slice::<Value>(&b).ok().and_then(|v| v.get("version")?.as_str().map(String::from))
            {
                return v;
            }
        }
        if let Ok(t) = fs::read_to_string(self.dir.join("pyproject.toml")) {
            // (The [project] table's version = "…", without a TOML parser.)
            let mut in_project = false;
            for line in t.lines() {
                let l = line.trim();
                if l.starts_with('[') {
                    in_project = l == "[project]" || l == "[tool.poetry]";
                } else if in_project {
                    if let Some(v) = l.strip_prefix("version").map(str::trim).and_then(|r| r.strip_prefix('=')) {
                        return v.trim().trim_matches(['"', '\'']).to_string();
                    }
                }
            }
        }
        String::new()
    }
}

/// Objects merged key by key; anything else replaced; null removes the key.
fn merge(base: &mut Value, over: Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                if v.is_null() {
                    b.remove(&k);
                } else if let Some(slot) = b.get_mut(&k) {
                    merge(slot, v);
                } else {
                    b.insert(k, v);
                }
            }
        }
        (b, o) => *b = o,
    }
}

fn web(command: Vec<String>, port: i64) -> Process {
    Process { name: "web".into(), command, port: Some(Port::http(port)), ..Default::default() }
}

fn defaults(kv: &[(&str, &str)]) -> Env {
    Env { defaults: kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), ..Default::default() }
}

fn npm_install(dir: &Path, production: bool) -> Result<()> {
    let verb = if exists(dir, "package-lock.json") { "ci" } else { "install" };
    let mut args = vec![verb, "--no-audit", "--no-fund"];
    if production {
        args.push("--omit=dev");
    }
    run(dir, &[], "npm", &args)
}

#[derive(Deserialize, Default)]
struct PackageJson {
    #[serde(default)]
    name: String,
    #[serde(default)]
    main: String,
    #[serde(default)]
    scripts: BTreeMap<String, String>,
    #[serde(default)]
    dependencies: BTreeMap<String, Value>,
    #[serde(default)]
    engines: Engines,
}

#[derive(Deserialize, Default)]
struct Engines {
    #[serde(default)]
    node: String,
}
