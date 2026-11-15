//! odi build: a project to an ODI file, with the project's own tools. What it
//! knows (the first that fits):
//!
//! - Go: go.mod: a static executable (CGO_ENABLED=0) for --platform
//! - Next.js: package.json with next: npm run build, .next/standalone in
//!   app/, .next/static and public/ in static/
//! - static: package.json whose build makes dist/ (or build/, out/) with
//!   index.html and no start script; or index.html at the top
//! - Node.js: package.json: the project and its production dependencies
//!   (npm ci --omit=dev), started by its start script or main
//! - Python: requirements.txt, pyproject.toml or a main file: the project
//!   and its packages (pip install --target app/.packages)
//!
//! A Procfile's web line, and odi.json in the project (any manifest fields),
//! override what was found. The build time is the last commit's (or
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
use crate::{copy_one, copy_tree, exists, host_arch, host_os, pack, walk, Step, VERSION};

pub struct Project {
    pub dir: PathBuf,
    pub stage: PathBuf,
    pub goos: String,
    pub goarch: String,
    pub build: bool,
    pub m: Manifest,
}

pub fn cmd_build(args: &[String]) -> Result<()> {
    const U: &str =
        "odi build [PROJECT] [-o FILE] [--platform OS/ARCH] [--no-build] [--runtime PART] [--compress max] [--profile [--path /]... [-e KEY=VALUE]...]";
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
    ];
    let (f, pos) = flags::parse(&specs, args, U)?;
    if pos.len() > 1 {
        bail!("usage: {U}");
    }
    crate::set_compress(f.str("compress"))?;
    let dir = std::path::absolute(pos.first().map(String::as_str).unwrap_or("."))?;
    let plat = f.str("platform").map(String::from).unwrap_or_else(|| format!("{}/{}", host_os(), host_arch()));
    let Some((goos, goarch)) = plat.split_once('/') else { bail!("--platform: OS/ARCH, e.g. linux/amd64") };
    let stage = tempfile::Builder::new().prefix("odi-build-").tempdir()?;
    let base = dir.file_name().and_then(|n| n.to_str()).unwrap_or("app");
    let mut p = Project {
        dir: dir.clone(),
        stage: stage.path().to_path_buf(),
        goos: goos.into(),
        goarch: goarch.into(),
        build: !f.bool("no-build"),
        m: Manifest { odi: "0.1".into(), name: safe_name(base), ..Default::default() },
    };

    let kind = if exists(&dir, "go.mod") {
        p.build_go()?;
        "Go".to_string()
    } else if exists(&dir, "package.json") {
        p.build_node()?
    } else if ["requirements.txt", "pyproject.toml", "app.py", "main.py"].iter().any(|f| exists(&dir, f)) {
        p.build_python()?;
        "Python".into()
    } else if exists(&dir, "index.html") {
        p.build_static(&dir)?;
        "static site".into()
    } else {
        bail!("no project here odi knows (go.mod, package.json, requirements.txt, pyproject.toml, index.html); pack a laid-out folder with odi pack");
    };
    p.procfile()?;
    p.overrides()?;
    p.build_info();
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
        let installed = exists(&self.dir, "node_modules");
        let script = |s: &str| pkg.scripts.get(s).map(String::as_str).unwrap_or("");
        if self.build && !script("build").is_empty() {
            if !installed {
                npm_install(&self.dir, false)?;
            }
            run(&self.dir, &[], "npm", &["run", "build"])?;
        }
        if pkg.dependencies.contains_key("next") {
            self.nextjs()?;
            return Ok("Next.js".into());
        }
        if script("start").is_empty() && pkg.main.is_empty() {
            for out in ["dist", "build", "out"] {
                let d = self.dir.join(out);
                if exists(&d, "index.html") {
                    self.m.runtime = None;
                    self.m.language = Some(Language { name: "static".into(), framework: String::new() });
                    self.build_static(&d)?;
                    return Ok(format!("static site ({out}/)"));
                }
            }
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

    fn node_version(&self, pkg: &PackageJson) -> String {
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
        if exists(&self.dir, ".next/static") {
            copy_tree(&self.dir.join(".next/static"), &self.stage.join("static/_next/static"))?;
            self.m.r#static.push(Static {
                from: "static/_next/static".into(),
                at: "/_next/static".into(),
                immutable: true,
            });
        }
        if exists(&self.dir, "public") {
            copy_tree(&self.dir.join("public"), &self.stage.join("static/public"))?;
            self.m.r#static.push(Static { from: "static/public".into(), at: "/".into(), immutable: false });
            // (and in the app too: its image optimizer reads originals from its own server)
            copy_tree(&self.dir.join("public"), &self.stage.join("app/public"))?;
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
        let Ok(b) = fs::read(self.dir.join("odi.json")) else { return Ok(()) };
        let over: Value = serde_json::from_slice(&b).context("odi.json")?;
        let mut cur = serde_json::to_value(&self.m)?;
        merge(&mut cur, over);
        self.m = serde_json::from_value(cur).context("odi.json")?;
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
        if !bi.revision.is_empty() && self.m.version.is_empty() {
            self.m.version = bi.revision.chars().take(12).collect();
        }
        self.m.build = Some(bi);
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
