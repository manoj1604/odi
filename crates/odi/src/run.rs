//! odi run: the runner contract (SPEC.md §7) on this machine.
//!
//! 1. The file is read and its digest checked, then its platform (§5.2).
//! 2. Parts (a set's, and a runtime pinned by digest) are put together into
//!    one tree (parts.rs). The runtime: the included or pinned one
//!    (runtime/bin first on PATH), or one on this machine's PATH whose
//!    version matches (§5.3). A runtime pinned by digest whose part isn't
//!    here is refused, unless --use-path-runtime.
//! 3. The files: unpacked once, every chunk checked, into a cache folder
//!    named by the digest (the same file runs again without unpacking). From
//!    a registry on Linux: mounted read-only, fetched as they're read.
//! 4. Each process starts in its workdir with env.defaults, -e values, PORT,
//!    ODI_ROOT, ODI_DIGEST, TMPDIR (a fresh folder) and PATH.
//! 5. Static files are served here, and other requests go to web; without
//!    static files, web takes the port itself.

use std::collections::{BTreeMap, HashSet};
use std::convert::Infallible;
use std::ffi::OsString;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use bytes::Bytes;
use http_body_util::{combinators::UnsyncBoxBody, BodyExt, Empty, Full};
use hyper::body::Incoming;
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::client::legacy::{connect::HttpConnector, Client};
use hyper_util::rt::{TokioExecutor, TokioIo};
use odi_format::{digest_string, Platform, Reader, Runtime, Static};
use tower::ServiceExt;

use crate::flags::{self, flag, Kind::*};
use crate::hints;
use crate::hub::Prefetch;
use crate::parts::{self, Assembled};
use crate::{host_arch, host_os, human, or_default};

pub fn cmd_run(args: &[String]) -> Result<()> {
    const U: &str =
        "odi run FILE|NAMESPACE/NAME[:TAG] [--port 8080] [-e KEY=VALUE]... [--use-path-runtime] [--trust KEY]...";
    let specs = [
        flag("trust", Multi, "run it only if this key signed it (repeat; also ~/.odi/trust, $ODI_TRUST)"),
        flag("port", Value, "the port to serve on (default: web's default, else 8080)"),
        flag("e", Multi, "KEY=VALUE for the program (repeat)"),
        flag("use-path-runtime", Bool, "run a runtime pinned by digest with this machine's, unchecked"),
    ];
    let (f, pos) = flags::parse(&specs, args, U)?;
    if pos.len() != 1 {
        bail!("usage: {U}");
    }
    let opts = RunOpts::from_flags(&f)?;
    let (rd, remote) = crate::hub::open_any(&pos[0])?;
    crate::keys::check_trusted(&rd, f.all("trust"))?;
    let app = parts::assemble(rd, remote)?;
    say_parts(&app);
    run_app(&app, &opts, None).map(|_| ())
}

/// odi profile: runs a file (mounted, Linux), asks it for some paths, and
/// keeps what it read, in order, as its start list (meta/odi/start.json): a
/// runner fetching it from a registry then gets all of it in one batch.
pub fn cmd_profile(args: &[String]) -> Result<()> {
    const U: &str = "odi profile FILE [-o FILE] [--path /]... [--port 8080] [-e KEY=VALUE]... [--use-path-runtime]\n       odi profile FILE --mcp [--input FILE] [-o FILE]   (a stdio tool)";
    let specs = [
        flag("mcp", Bool, "it's a stdio MCP server: ask it initialize and tools/list (or --input), not URL paths"),
        flag("input", Value, "with --mcp: a file of JSON-RPC lines to send it instead"),
        flag("o", Value, "the ODI file to write (default: FILE, replaced)"),
        flag("path", Multi, "a URL path to ask for once it's up (repeat; default /)"),
        flag("port", Value, "the port to serve on (default: web's default, else 8080)"),
        flag("e", Multi, "KEY=VALUE for the program (repeat)"),
        flag("use-path-runtime", Bool, "run a runtime pinned by digest with this machine's, unchecked"),
    ];
    let (f, pos) = flags::parse(&specs, args, U)?;
    if pos.len() != 1 {
        bail!("usage: {U}");
    }
    let opts = RunOpts::from_flags(&f)?;
    let out = PathBuf::from(f.str("o").unwrap_or(&pos[0]));
    if f.bool("mcp") || f.str("input").is_some() {
        let lines = match f.str("input") {
            Some(i) => fs::read_to_string(i).with_context(|| i.to_string())?.lines().map(String::from).collect(),
            None => MCP_HELLO.iter().map(|l| l.to_string()).collect(),
        };
        return profile_stdio(Path::new(&pos[0]), &out, lines, &opts);
    }
    profile(Path::new(&pos[0]), &out, f.all("path"), &opts)
}

/// What an agent first asks an MCP server.
const MCP_HELLO: [&str; 3] = [
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"odi-profile","version":"1"}}}"#,
    r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
];

/// Profiles a stdio tool: runs what it provides as an mcp plug (else its
/// first process), mounted, sends it lines, waits for an answer to each one
/// with an id, and keeps what it read as its start list.
fn profile_stdio(path: &Path, out: &Path, lines: Vec<String>, opts: &RunOpts) -> Result<()> {
    use std::io::{BufRead, Write};
    let app = parts::assemble(Reader::open(path)?, None)?;
    let pick = if app.rd.manifest.provides.iter().any(|g| g.plug == "mcp")
        || app.parts.iter().any(|u| u.part.role.is_empty() && u.manifest.provides.iter().any(|g| g.plug == "mcp"))
    {
        Pick::Plug("mcp", None)
    } else {
        Pick::Process(None)
    };
    let (from, pm, process) = pick_process(&app, None, &pick)?;
    let prep = prepare(&app, opts, true)?;
    let proot = match &from {
        Some(n) => prep.root.join("parts").join(n),
        None => prep.root.clone(),
    };
    let wd = proot.join(or_default(&process.workdir, "app"));
    let prog = look_program(&process.command[0], &proot, &wd, &prep.path_env);
    let mut child = std::process::Command::new(&prog)
        .args(&process.command[1..])
        .current_dir(&wd)
        .envs(prep.env(pm.env.as_ref(), &[]))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .with_context(|| process.command[0].clone())?;
    let want = lines
        .iter()
        .filter(|l| serde_json::from_str::<serde_json::Value>(l).is_ok_and(|v| v.get("id").is_some()))
        .count();
    let mut stdin = child.stdin.take().expect("piped");
    for l in &lines {
        writeln!(stdin, "{l}")?;
    }
    stdin.flush()?;
    // (Answers read on their own thread, so a tool that never answers times out.)
    let stdout = child.stdout.take().expect("piped");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for l in std::io::BufReader::new(stdout).lines() {
            let Ok(l) = l else { break };
            if serde_json::from_str::<serde_json::Value>(&l).is_ok_and(|v| v.get("id").is_some())
                && tx.send(()).is_err()
            {
                break;
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut got = 0;
    while got < want {
        let left = deadline.saturating_duration_since(Instant::now());
        if rx.recv_timeout(left).is_err() {
            let _ = child.kill();
            bail!("it answered {got} of {want} requests within a minute");
        }
        got += 1;
    }
    eprintln!("odi: it answered all {want} requests");
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    let reads = prep.mounted.as_ref().map(|mnt| mnt.reads(&prep.rd)).unwrap_or_default();
    drop(prep);
    let rd = &app.rd;
    let start = hints::StartList::from_reads(&reads, rd);
    let bytes: u64 = start.ranges(rd).iter().map(|r| r.1).sum();
    let d = crate::repack(&Reader::open(path)?, &[(hints::START, serde_json::to_vec(&start)?)], out)?;
    println!(
        "{} {}  start list: {} files, {} stored",
        out.display(),
        digest_string(&d),
        start.files.len(),
        human(bytes)
    );
    Ok(())
}

/// Profiles the ODI file at path into out (they may be the same): runs it,
/// asks it for each URL path (default /), and stores what it read as its
/// start list.
pub fn profile(path: &Path, out: &Path, paths: &[String], opts: &RunOpts) -> Result<()> {
    let mut paths = paths.to_vec();
    if paths.is_empty() {
        paths.push("/".into());
    }
    // (A set is run whole, and the start list goes in the set's own file: its
    // paths are the whole tree's.)
    let app = parts::assemble(Reader::open(path)?, None)?;
    say_parts(&app);
    let reads = run_app(&app, opts, Some(paths))?;
    let rd = &app.rd;
    let start = hints::StartList::from_reads(&reads, rd);
    let bytes: u64 = start.ranges(rd).iter().map(|r| r.1).sum();
    let json = serde_json::to_vec(&start)?;
    let d = crate::repack(&Reader::open(path)?, &[(hints::START, json)], out)?;
    println!(
        "{} {}  start list: {} files, {} stored",
        out.display(),
        digest_string(&d),
        start.files.len(),
        human(bytes)
    );
    Ok(())
}

/// Says which parts a set was put together from.
fn say_parts(app: &Assembled) {
    for u in &app.parts {
        let role = if u.part.role.is_empty() { "part".to_string() } else { u.part.role.clone() };
        let at = if u.remote { "from the registry" } else { "here" };
        eprintln!("odi: {role} {} {} ({}, {at})", u.part.name, short(&u.part.digest), human(u.size));
    }
    if !app.replaced.is_empty() {
        eprintln!("odi: {} files replaced by later parts (odi inspect lists them)", app.replaced.len());
    }
}

pub fn short(d: &str) -> &str {
    let h = d.strip_prefix("sha256:").unwrap_or(d);
    &h[..h.len().min(12)]
}

#[derive(Clone)]
pub struct RunOpts {
    port: Option<i64>,
    envs: Vec<String>,
    path_runtime: bool,
    /// Keys a part added while running must be signed by (with the runner's).
    trust: Vec<String>,
}

impl RunOpts {
    pub fn from_flags(f: &flags::Flags) -> Result<RunOpts> {
        Ok(RunOpts {
            port: f.int("port")?,
            envs: f.all("e").to_vec(),
            path_runtime: f.bool("use-path-runtime"),
            trust: f.all("trust").to_vec(),
        })
    }
}

/// Runs a file: from a registry or profiling, mounted (Linux), else
/// unpacked. Profiling (`profile`: the paths to ask for), it stops once each
/// has answered and returns what was read, in order.
/// A file ready to run: its tree (mounted or unpacked), PATH, and the
/// values given for its environment.
struct Prepared {
    rd: Arc<Reader>,
    remote: Option<Arc<dyn Prefetch>>,
    m: odi_format::Manifest,
    tmp: tempfile::TempDir,
    mounted: Option<crate::mount::Mounted>,
    root: PathBuf,
    path_env: OsString,
    given: BTreeMap<String, String>,
    routes: Option<Arc<RouteHints>>,
}

/// Gets a file ready: from a registry or recording reads, mounted (Linux),
/// else unpacked; what it reads while starting fetched ahead in one batch.
fn prepare(app: &Assembled, opts: &RunOpts, record: bool) -> Result<Prepared> {
    let (rd, remote) = (app.rd.clone(), app.remote.clone());
    let m = rd.manifest.clone();
    check_platform(m.platform.as_ref())?;
    let tmp = tempfile::Builder::new().prefix("odi-tmp-").tempdir()?;

    let mut routes = None;
    let mut warm = Vec::new();
    if let Some(pr) = &remote {
        // What the program reads while starting, in one batch, before it starts.
        if let Some(start) = hints::load::<hints::StartList>(&rd, hints::START) {
            let (t, before) = (Instant::now(), pr.stats().requests);
            pr.prefetch(&start.ranges(&rd));
            for f in &start.files {
                let Some(e) = rd.entry_index(&f.path) else { continue };
                if f.chunks.is_empty() {
                    warm.extend((0..rd.entries[e].chunks.len()).map(|c| (e, c)));
                } else {
                    warm.extend(f.chunks.iter().map(|&c| (e, c)));
                }
            }
            eprintln!(
                "odi: start list: {} fetched ahead in {} request(s), {} ms",
                human(pr.stats().fetched),
                pr.stats().requests - before,
                t.elapsed().as_millis()
            );
        }
        if let Some(map) = hints::load::<hints::RouteMap>(&rd, hints::ROUTES) {
            routes = Some(Arc::new(RouteHints { map, rd: rd.clone(), pr: pr.clone(), done: Mutex::default() }));
        }
    }

    let mut mounted = None;
    if remote.is_some() || record {
        match crate::mount::mount_lazy(rd.clone(), &tmp.path().join(".writable"), remote.clone(), record, warm) {
            Ok(mnt) => mounted = Some(mnt),
            Err(e) if record => return Err(e.context("profiling reads the file mounted (Linux, FUSE)")),
            Err(e) => eprintln!("odi: getting the whole file ({e:#})"),
        }
    }
    let root = match &mounted {
        Some(mnt) => mnt.dir.clone(),
        None => unpack_cached(&rd, &app.key)?,
    };

    let mut path_env = std::env::var_os("PATH").unwrap_or_default();
    let mut first = Vec::new();
    if let Some(rt) = &m.runtime {
        if rt.included {
            first.push(root.join("runtime").join("bin"));
        } else {
            if !rt.digest.is_empty() && !opts.path_runtime {
                bail!("{} {} is pinned by digest ({}), and that runtime part isn't here: odi part add FILE, or put it in a set (odi set --runtime NAME); --use-path-runtime runs it with this machine's, unchecked", rt.name, rt.version, rt.digest);
            }
            check_runtime(rt)?;
        }
    }
    // (Parts that provide a command: their app/bin, on everyone's PATH.)
    for u in &app.parts {
        if u.part.role.is_empty() && u.manifest.provides.iter().any(|g| g.plug == "cli") {
            first.push(root.join("parts").join(&u.part.name).join("app").join("bin"));
        }
    }
    if !first.is_empty() {
        first.extend(std::env::split_paths(&path_env));
        path_env = std::env::join_paths(first)?;
    }

    let mut given: BTreeMap<String, String> = BTreeMap::new();
    for kv in &opts.envs {
        match kv.split_once('=') {
            Some((k, v)) if !k.is_empty() => {
                given.insert(k.into(), v.into());
            }
            _ => bail!("-e {kv:?}: KEY=VALUE"),
        }
    }
    if let Some(env) = &m.env {
        let mut missing = Vec::new();
        for k in &env.required {
            if !given.contains_key(k) {
                match std::env::var(k) {
                    Ok(v) => {
                        given.insert(k.clone(), v);
                    }
                    Err(_) => missing.push(k.as_str()),
                }
            }
        }
        if !missing.is_empty() {
            bail!("it needs {}: -e {}=...", missing.join(", "), missing[0]);
        }
        for k in &env.optional {
            if let (Ok(v), false) = (std::env::var(k), given.contains_key(k)) {
                given.insert(k.clone(), v);
            }
        }
    }
    Ok(Prepared { rd, remote, m, tmp, mounted, root, path_env, given, routes })
}

impl Prepared {
    /// A process's environment: the manifest's defaults (of the file the
    /// process is from), what was given, and what the runner says.
    fn env(&self, defaults: Option<&odi_format::Env>, plugs: &[(String, String)]) -> Vec<(String, OsString)> {
        let mut env: Vec<(String, OsString)> = Vec::new();
        if let Some(e) = defaults {
            env.extend(e.defaults.iter().map(|(k, v)| (k.clone(), v.into())));
        }
        env.extend(self.given.iter().map(|(k, v)| (k.clone(), v.into())));
        env.extend(plugs.iter().map(|(k, v)| (k.clone(), v.into())));
        env.push(("ODI_ROOT".into(), self.root.clone().into()));
        env.push(("ODI_DIGEST".into(), digest_string(&self.rd.digest).into()));
        env.push(("TMPDIR".into(), self.tmp.path().into()));
        env.push(("PATH".into(), self.path_env.clone()));
        env
    }

    fn say_fetched(&self) {
        if let Some(pr) = &self.remote {
            let s = pr.stats();
            let local = if s.cached > 0 {
                format!(", {} from this machine's piece cache", human(s.cached))
            } else {
                String::new()
            };
            eprintln!("odi: fetched {} of {} in {} requests{local}", human(s.fetched), human(s.size), s.requests);
        }
    }
}

/// A plug's name as an environment variable's: `ODI_PLUG_NAME_URL`.
pub fn plug_var(name: &str) -> String {
    let n: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect();
    format!("ODI_PLUG_{n}_URL")
}

/// The services a set's parts provide (`http`, `tcp` plugs): a process each,
/// on a port of its own, and where to reach it for everyone else.
struct Service {
    part: String,
    process: odi_format::Process,
    env: Option<odi_format::Env>,
    port: u16,
}

/// Environment variables: (name, value).
type Vars = Vec<(String, String)>;

fn services(app: &Assembled) -> Result<(Vec<Service>, Vars)> {
    let (mut out, mut plugs) = (Vec::new(), Vec::new());
    for u in app.parts.iter().filter(|u| u.part.role.is_empty()) {
        for g in u.manifest.provides.iter().filter(|g| g.plug == "http" || g.plug == "tcp") {
            let pm = &u.manifest;
            let p = if g.process.is_empty() {
                pm.processes.first()
            } else {
                pm.processes.iter().find(|p| p.name == g.process)
            };
            let Some(p) = p else {
                bail!("part {} provides {} {}, but has no process for it", u.part.name, g.plug, g.name)
            };
            if out.iter().any(|s: &Service| s.part == u.part.name && s.process.name == p.name) {
                continue;
            }
            let port = free_port()?;
            let scheme = if g.plug == "http" { "http" } else { "tcp" };
            plugs.push((plug_var(&g.name), format!("{scheme}://127.0.0.1:{port}")));
            out.push(Service { part: u.part.name.clone(), process: p.clone(), env: pm.env.clone(), port });
        }
    }
    // What's needed must be provided, by a part or by the set itself.
    let given: Vec<(&str, &str)> = app
        .parts
        .iter()
        .flat_map(|u| u.manifest.provides.iter())
        .chain(&app.rd.manifest.provides)
        .map(|g| (g.plug.as_str(), g.name.as_str()))
        .collect();
    for n in app.rd.manifest.needs.iter().chain(app.parts.iter().flat_map(|u| u.manifest.needs.iter())) {
        if n.plug != "runtime" && !given.contains(&(n.plug.as_str(), n.name.as_str())) {
            bail!("it needs {} {}, and no part provides it", n.plug, n.name);
        }
    }
    Ok((out, plugs))
}

/// Runs a file: from a registry or profiling, mounted (Linux), else
/// unpacked. Profiling (`profile`: the paths to ask for), it stops once each
/// has answered and returns what was read, in order.
fn run_app(app: &Assembled, opts: &RunOpts, profile: Option<Vec<String>>) -> Result<Vec<(String, usize)>> {
    let (services, plugs) = services(app)?;
    let prep = prepare(app, opts, profile.is_some())?;
    let (m, root) = (&prep.m, &prep.root);

    let web = m.processes.iter().find(|p| p.name == "web");
    let port = match opts.port {
        Some(p) if p != 0 => p,
        _ => web.and_then(|w| w.port.as_ref()).map(|p| p.default).filter(|d| *d != 0).unwrap_or(8080),
    };
    let port = u16::try_from(port).map_err(|_| anyhow!("--port {port}: not a port"))?;
    let web_port = if web.is_some() && !m.r#static.is_empty() { free_port()? } else { port };

    let digest = digest_string(&prep.rd.digest);
    // Every plug, where to reach it, in a file every process is told of
    // (ODI_PLUGS): rewritten as parts are added while it runs.
    let live = Arc::new(Live {
        file: prep.tmp.path().join("plugs.json"),
        plugs: Mutex::new(Vec::new()),
        held: Mutex::new(Vec::new()),
        opts: opts.clone(),
        spawn: Mutex::new(None),
    });
    let mut plugs = plugs;
    plugs.push(("ODI_PLUGS".into(), live.file.display().to_string()));
    for u in &app.parts {
        for g in &u.manifest.provides {
            let url = plugs.iter().find(|(k, _)| *k == plug_var(&g.name)).map(|(_, v)| v.clone());
            live.plugs.lock().expect("lock").push(PlugInfo {
                plug: g.plug.clone(),
                name: g.name.clone(),
                part: u.part.name.clone(),
                url,
                root: Some(root.join("parts").join(&u.part.name).display().to_string()),
            });
        }
    }
    live.write()?;
    let mut procs = Vec::new();
    for s in &services {
        let proot = root.join("parts").join(&s.part);
        let mut env = prep.env(s.env.as_ref(), &plugs);
        let pe = s.process.port.as_ref().map(|p| or_default(&p.env, "PORT")).unwrap_or("PORT");
        env.push((pe.into(), s.port.to_string().into()));
        let wd = proot.join(or_default(&s.process.workdir, "app"));
        let prog = look_program(&s.process.command[0], &proot, &wd, &prep.path_env);
        procs.push(Proc {
            name: format!("{}/{}", s.part, s.process.name),
            shown: s.process.command.join(" "),
            prog,
            args: s.process.command[1..].to_vec(),
            wd,
            env,
            essential: true,
        });
    }
    for pr in &m.processes {
        let mut env = prep.env(m.env.as_ref(), &plugs);
        if let Some(p) = &pr.port {
            env.push((or_default(&p.env, "PORT").into(), web_port.to_string().into()));
        }
        let wd = root.join(or_default(&pr.workdir, "app"));
        let prog = look_program(&pr.command[0], root, &wd, &prep.path_env);
        procs.push(Proc {
            name: pr.name.clone(),
            shown: pr.command.join(" "),
            prog,
            args: pr.command[1..].to_vec(),
            wd,
            env,
            essential: true,
        });
    }
    let site = (!m.r#static.is_empty()).then(|| Site {
        root: root.clone(),
        statics: m.r#static.clone(),
        web: web.map(|_| Web { port: web_port, client: Client::builder(TokioExecutor::new()).build_http() }),
        routes: prep.routes.clone(),
    });
    let health = web.and_then(|w| w.health.as_ref()).map(|h| h.path.clone());

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let control = if profile.is_some() { None } else { Some(live.clone()) };
    let until = profile.map(|paths| Box::pin(answered(port, paths)) as Until);
    let r = rt.block_on(supervise(procs, site, port, web_port, health, &m.name, &digest, until, control));
    drop(rt);
    live.held.lock().expect("lock").clear();
    let reads = prep.mounted.as_ref().map(|mnt| mnt.reads(&prep.rd)).unwrap_or_default();
    prep.say_fetched();
    drop(prep);
    r.map(|_| reads)
}

/// odi exec: runs one of a file's processes (or a part's), its input and
/// output this terminal's: a command-line tool, or a stdio MCP server.
pub fn cmd_exec(args: &[String]) -> Result<()> {
    const U: &str = "odi exec FILE|NAME [--process NAME] [--part NAME] [-e KEY=VALUE]... [-- ARGS...]";
    let (args, extra) = match args.iter().position(|a| a == "--") {
        Some(i) => (&args[..i], args[i + 1..].to_vec()),
        None => (args, Vec::new()),
    };
    let specs = [
        flag("process", Value, "which process (default: the only one, or the first)"),
        flag("part", Value, "a part of the set's, not the set's own"),
        flag("e", Multi, "KEY=VALUE for the program (repeat)"),
        flag("use-path-runtime", Bool, "run a runtime pinned by digest with this machine's, unchecked"),
        flag("trust", Multi, "run it only if this key signed it (repeat; also ~/.odi/trust, $ODI_TRUST)"),
    ];
    let (f, pos) = flags::parse(&specs, args, U)?;
    if pos.len() != 1 {
        bail!("usage: {U}");
    }
    let code = exec(&pos[0], f.str("part"), Pick::Process(f.str("process").map(String::from)), &extra, &f)?;
    std::process::exit(code)
}

/// odi mcp: runs what a file (or one of a set's parts) provides as an `mcp`
/// plug, as a stdio MCP server: what an agent's configuration names as its
/// command (`odi mcp acme/pdf:1`).
pub fn cmd_mcp(args: &[String]) -> Result<()> {
    const U: &str = "odi mcp FILE|NAME [--name PLUG] [-e KEY=VALUE]...";
    let specs = [
        flag("name", Value, "which mcp plug, when there are several"),
        flag("e", Multi, "KEY=VALUE for the program (repeat)"),
        flag("use-path-runtime", Bool, "run a runtime pinned by digest with this machine's, unchecked"),
        flag("trust", Multi, "run it only if this key signed it (repeat; also ~/.odi/trust, $ODI_TRUST)"),
    ];
    let (f, pos) = flags::parse(&specs, args, U)?;
    if pos.len() != 1 {
        bail!("usage: {U}");
    }
    let code = exec(&pos[0], None, Pick::Plug("mcp", f.str("name").map(String::from)), &[], &f)?;
    std::process::exit(code)
}

/// Which process: a file's own or one of its parts' (under parts/NAME/),
/// by name or by the plug it serves.
fn pick_process(
    app: &Assembled,
    part: Option<&str>,
    pick: &Pick,
) -> Result<(Option<String>, odi_format::Manifest, odi_format::Process)> {
    // Whose process: the set's own, or a part's (under parts/NAME/).
    let mut candidates: Vec<(Option<&str>, &odi_format::Manifest)> = vec![(None, &app.rd.manifest)];
    candidates
        .extend(app.parts.iter().filter(|u| u.part.role.is_empty()).map(|u| (Some(u.part.name.as_str()), &u.manifest)));
    if let Some(p) = part {
        candidates.retain(|(n, _)| *n == Some(p));
        if candidates.is_empty() {
            bail!("no part {p} (under parts/)");
        }
    }
    let (from, pm, process) = match pick {
        Pick::Process(want) => {
            let (from, pm) = candidates[0];
            let p = match want {
                Some(w) => pm.processes.iter().find(|p| &p.name == w),
                None => pm.processes.first(),
            };
            let Some(p) = p else { bail!("no process {}", want.as_deref().unwrap_or("to run")) };
            (from, pm, p.clone())
        }
        Pick::Plug(kind, want) => {
            let mut found = Vec::new();
            for (from, pm) in &candidates {
                for g in pm.provides.iter().filter(|g| g.plug == *kind && want.as_ref().is_none_or(|w| &g.name == w)) {
                    let p = if g.process.is_empty() {
                        pm.processes.iter().find(|p| p.name == *kind).or(pm.processes.first())
                    } else {
                        pm.processes.iter().find(|p| p.name == g.process)
                    };
                    if let Some(p) = p {
                        found.push((*from, *pm, p.clone(), g.name.clone()));
                    }
                }
            }
            match found.len() {
                0 => bail!("it provides no {kind} plug{}", want.as_ref().map(|w| format!(" {w}")).unwrap_or_default()),
                1 => {
                    let (a, b, c, _) = found.remove(0);
                    (a, b, c)
                }
                _ => bail!(
                    "it provides several {kind} plugs ({}): --name one",
                    found.iter().map(|x| x.3.as_str()).collect::<Vec<_>>().join(", ")
                ),
            }
        }
    };
    Ok((from.map(String::from), pm.clone(), process))
}

enum Pick {
    Process(Option<String>),
    Plug(&'static str, Option<String>),
}

fn exec(src: &str, part: Option<&str>, pick: Pick, extra: &[String], f: &flags::Flags) -> Result<i32> {
    let opts = RunOpts {
        port: None,
        envs: f.all("e").to_vec(),
        path_runtime: f.bool("use-path-runtime"),
        trust: f.all("trust").to_vec(),
    };
    let (rd, remote) = crate::hub::open_any(src)?;
    crate::keys::check_trusted(&rd, f.all("trust"))?;
    let app = parts::assemble(rd, remote)?;
    let (from, pm, process) = pick_process(&app, part, &pick)?;
    let (from, pm) = (from.as_deref(), &pm);
    let (services, plugs) = services(&app)?;
    if !services.is_empty() {
        bail!("odi exec runs one process; this set's parts provide services: odi run it");
    }
    let prep = prepare(&app, &opts, false)?;
    let proot = match from {
        Some(n) => prep.root.join("parts").join(n),
        None => prep.root.clone(),
    };
    let wd = proot.join(or_default(&process.workdir, "app"));
    let prog = look_program(&process.command[0], &proot, &wd, &prep.path_env);
    let st = std::process::Command::new(&prog)
        .args(&process.command[1..])
        .args(extra)
        .current_dir(&wd)
        .envs(prep.env(pm.env.as_ref(), &plugs))
        .status()
        .with_context(|| process.command[0].clone())?;
    if std::env::var_os("ODI_VERBOSE").is_some() {
        prep.say_fetched();
    }
    drop(prep);
    Ok(st.code().unwrap_or(1))
}

/// Something that ends a run early, cleanly.
type Until = std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>;

/// Each path answered (any status but a gateway error), within two minutes;
/// a page, then every local file it names, as a browser loading it would ask
/// (so what a first visit makes the app read, an image resized say, is kept
/// too).
async fn answered(port: u16, paths: Vec<String>) -> Result<()> {
    let client: Client<HttpConnector, Empty<Bytes>> = Client::builder(TokioExecutor::new()).build_http();
    let get = |path: String| {
        let client = client.clone();
        async move {
            let uri: hyper::Uri = format!("http://127.0.0.1:{port}{path}").parse()?;
            let req = Request::get(uri)
                .header(header::ACCEPT, "text/html,image/avif,image/webp,*/*")
                .body(Empty::<Bytes>::new())?;
            let res = client.request(req).await?;
            let status = res.status();
            let html = res.headers().get(header::CONTENT_TYPE).is_some_and(|v| v.as_bytes().starts_with(b"text/html"));
            let body = res.into_body().collect().await?.to_bytes();
            anyhow::Ok((status, html.then(|| String::from_utf8_lossy(&body).into_owned())))
        }
    };
    let deadline = Instant::now() + Duration::from_secs(120);
    for p in paths {
        let html = loop {
            if let Ok((status, html)) = get(p.clone()).await {
                if !matches!(status.as_u16(), 502..=504) {
                    eprintln!("odi: {p} answered {status}");
                    break html;
                }
            }
            if Instant::now() > deadline {
                bail!("{p} didn't answer within two minutes");
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        let Some(html) = html else { continue };
        let base = &p[..p.find('?').unwrap_or(p.len())];
        let base = &base[..base.rfind('/').map_or(0, |i| i + 1)];
        let mut asked = HashSet::new();
        let assets: Vec<String> = hints::links(&html)
            .map(|l| l.replace("&amp;", "&"))
            .map(|l| if l.starts_with('/') { l } else { format!("{base}{l}") })
            .filter(|l| asked.insert(l.clone()))
            .collect();
        let n = assets.len();
        let gets: Vec<_> = assets.into_iter().map(|a| get(a.clone())).collect();
        for r in futures_join(gets).await {
            let _ = r;
        }
        eprintln!("odi: and the {n} files it names");
    }
    Ok(())
}

/// Runs futures at once, a few at a time as a browser does, and waits for all.
async fn futures_join<F: std::future::Future + Send + 'static>(fs: Vec<F>) -> Vec<F::Output>
where
    F::Output: Send + 'static,
{
    let mut out = Vec::new();
    let mut fs = fs.into_iter();
    loop {
        let wave: Vec<_> = fs.by_ref().take(6).map(tokio::spawn).collect();
        if wave.is_empty() {
            return out;
        }
        for h in wave {
            if let Ok(v) = h.await {
                out.push(v);
            }
        }
    }
}

/// A web app's route map, from a registry: the first request to a route
/// fetches all of its files in one batch.
pub struct RouteHints {
    map: hints::RouteMap,
    rd: Arc<Reader>,
    pr: Arc<dyn Prefetch>,
    done: Mutex<HashSet<usize>>,
}

impl RouteHints {
    fn fetch(&self, i: usize) {
        let files = self.map.routes[i].files.iter().map(|f| (f.as_str(), &[][..]));
        self.pr.prefetch(&hints::ranges(&self.rd, files));
    }
}

struct Proc {
    name: String,
    shown: String,
    prog: PathBuf,
    args: Vec<String>,
    wd: PathBuf,
    env: Vec<(String, OsString)>,
    /// Its exit ends the run (a part added while running: only itself).
    essential: bool,
}

// --- parts added while running ---------------------------------------------------

/// One plug, as ODI_PLUGS lists it.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PlugInfo {
    plug: String,
    name: String,
    part: String,
    /// Where its service listens (http, tcp).
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    /// Where its files are.
    #[serde(skip_serializing_if = "Option::is_none")]
    root: Option<String>,
}

/// A running set: its plugs, and what's needed to add parts to it.
struct Live {
    file: PathBuf,
    plugs: Mutex<Vec<PlugInfo>>,
    /// Parts added while running, mounted or unpacked, kept until it ends.
    held: Mutex<Vec<(String, Prepared)>>,
    opts: RunOpts,
    spawn: Mutex<Option<tokio::sync::mpsc::UnboundedSender<Proc>>>,
}

#[derive(serde::Deserialize)]
struct AttachAsk {
    part: String,
    #[serde(default)]
    name: String,
}

impl Live {
    fn write(&self) -> Result<()> {
        let plugs = self.plugs.lock().expect("lock").clone();
        let tmp = self.file.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&serde_json::json!({ "plugs": plugs }))?)?;
        fs::rename(&tmp, &self.file)?;
        Ok(())
    }

    /// Adds a part (a file, or a name in a registry): its files ready (from a
    /// registry, mounted and read as needed), its services started, its plugs
    /// listed. Returns what it provides.
    fn attach(&self, ask: &AttachAsk) -> Result<Vec<PlugInfo>> {
        let (rd, remote) = crate::hub::open_any(&ask.part)?;
        crate::keys::check_trusted(&rd, &self.opts.trust)?;
        let name = if ask.name.is_empty() { rd.manifest.name.clone() } else { ask.name.clone() };
        if !odi_format::valid_name(&name) {
            bail!("part name {name:?}: letters, digits, . - _");
        }
        if self.held.lock().expect("lock").iter().any(|(n, _)| *n == name)
            || self.plugs.lock().expect("lock").iter().any(|p| p.part == name)
        {
            bail!("a part named {name} is here already");
        }
        let app = parts::assemble(rd, remote)?;
        let prep = prepare(&app, &self.opts, false)?;
        let pm = prep.m.clone();
        let mut added = Vec::new();
        let mut procs = Vec::new();
        let env_now: Vars = self
            .plugs
            .lock()
            .expect("lock")
            .iter()
            .filter_map(|p| p.url.clone().map(|u| (plug_var(&p.name), u)))
            .chain([("ODI_PLUGS".to_string(), self.file.display().to_string())])
            .collect();
        for g in &pm.provides {
            let mut info = PlugInfo {
                plug: g.plug.clone(),
                name: g.name.clone(),
                part: name.clone(),
                url: None,
                root: Some(prep.root.display().to_string()),
            };
            if g.plug == "http" || g.plug == "tcp" {
                let p = if g.process.is_empty() {
                    pm.processes.first()
                } else {
                    pm.processes.iter().find(|p| p.name == g.process)
                };
                let Some(p) = p else { bail!("it provides {} {}, but has no process for it", g.plug, g.name) };
                let port = free_port()?;
                let scheme = if g.plug == "http" { "http" } else { "tcp" };
                info.url = Some(format!("{scheme}://127.0.0.1:{port}"));
                let mut env = prep.env(pm.env.as_ref(), &env_now);
                let pe = p.port.as_ref().map(|p| or_default(&p.env, "PORT")).unwrap_or("PORT");
                env.push((pe.into(), port.to_string().into()));
                let wd = prep.root.join(or_default(&p.workdir, "app"));
                procs.push(Proc {
                    name: format!("{name}/{}", p.name),
                    shown: p.command.join(" "),
                    prog: look_program(&p.command[0], &prep.root, &wd, &prep.path_env),
                    args: p.command[1..].to_vec(),
                    wd,
                    env,
                    essential: false,
                });
            }
            added.push(info);
        }
        let spawn = self.spawn.lock().expect("lock").clone().ok_or_else(|| anyhow!("not running"))?;
        for p in procs {
            spawn.send(p).map_err(|_| anyhow!("it's stopping"))?;
        }
        self.plugs.lock().expect("lock").extend(added.iter().cloned());
        self.held.lock().expect("lock").push((name.clone(), prep));
        self.write()?;
        eprintln!("odi: part {name} added ({} plugs)", added.len());
        Ok(added)
    }
}

/// Where a running set takes parts: a socket named after it.
fn control_path(name: &str) -> Result<PathBuf> {
    Ok(cache_base()?.join("run").join(format!("{name}.sock")))
}

/// Listens for `odi attach` (Unix): one JSON line asked, one answered.
#[cfg(unix)]
async fn control(live: Arc<Live>, name: String) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let Ok(path) = control_path(&name) else { return };
    let _ = fs::create_dir_all(path.parent().expect("in a folder"));
    if tokio::net::UnixStream::connect(&path).await.is_ok() {
        eprintln!("odi: another {name} is running: odi attach goes to it");
        return;
    }
    let _ = fs::remove_file(&path);
    let Ok(listener) = tokio::net::UnixListener::bind(&path) else { return };
    let _ = crate::set_mode(&path, 0o600);
    let _guard = RemoveOnDrop(path);
    loop {
        let Ok((conn, _)) = listener.accept().await else { return };
        let live = live.clone();
        tokio::spawn(async move {
            let (r, mut w) = conn.into_split();
            let mut line = String::new();
            if BufReader::new(r).read_line(&mut line).await.is_err() {
                return;
            }
            let answer = match serde_json::from_str::<AttachAsk>(&line) {
                Err(e) => serde_json::json!({ "error": format!("{e}") }),
                Ok(ask) => match tokio::task::spawn_blocking(move || live.attach(&ask)).await {
                    Ok(Ok(plugs)) => serde_json::json!({ "plugs": plugs }),
                    Ok(Err(e)) => serde_json::json!({ "error": format!("{e:#}") }),
                    Err(e) => serde_json::json!({ "error": format!("{e}") }),
                },
            };
            let _ = w.write_all(format!("{answer}\n").as_bytes()).await;
        });
    }
}

#[cfg(not(unix))]
async fn control(_: Arc<Live>, _: String) {}

struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// odi attach: adds a part to a set that's running (odi run NAME), which
/// starts what it provides and lists it in the file ODI_PLUGS names.
pub fn cmd_attach(args: &[String]) -> Result<()> {
    const U: &str = "odi attach RUNNING PART [--as NAME]   (RUNNING: the running set's name)";
    let specs = [flag("as", Value, "the part's name in the set (default: its own)")];
    let (f, pos) = flags::parse(&specs, args, U)?;
    if pos.len() != 2 {
        bail!("usage: {U}");
    }
    #[cfg(unix)]
    {
        use std::io::{BufRead, Write};
        let path = control_path(&pos[0])?;
        let mut c = std::os::unix::net::UnixStream::connect(&path)
            .map_err(|_| anyhow!("no {} running here (odi run it first)", pos[0]))?;
        // (A part from a file: by its full path, the runner's folder may differ.)
        let part =
            if Path::new(&pos[1]).exists() { fs::canonicalize(&pos[1])?.display().to_string() } else { pos[1].clone() };
        let ask = serde_json::json!({ "part": part, "name": f.str("as").unwrap_or("") });
        writeln!(c, "{ask}")?;
        let mut line = String::new();
        std::io::BufReader::new(c).read_line(&mut line)?;
        let v: serde_json::Value = serde_json::from_str(&line).context("the runner's answer")?;
        if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
            bail!("{e}");
        }
        for p in v["plugs"].as_array().into_iter().flatten() {
            let at = p.get("url").and_then(|u| u.as_str()).unwrap_or("");
            println!(
                "{} {} {} {at}",
                p["part"].as_str().unwrap_or(""),
                p["plug"].as_str().unwrap_or(""),
                p["name"].as_str().unwrap_or("")
            );
        }
        Ok(())
    }
    #[cfg(not(unix))]
    bail!("odi attach: Unix only, for now")
}

/// Starts a process, its exit told on tx (if it's essential to the run).
fn start_proc(
    p: Proc,
    waits: &mut tokio::task::JoinSet<()>,
    pids: &mut Vec<u32>,
    tx: &tokio::sync::mpsc::UnboundedSender<String>,
) -> Result<()> {
    let mut c = tokio::process::Command::new(&p.prog);
    c.args(&p.args).current_dir(&p.wd).envs(p.env).kill_on_drop(true);
    let mut child = c.spawn().with_context(|| p.name.clone())?;
    eprintln!("odi: {} started ({})", p.name, p.shown);
    pids.extend(child.id());
    let tx = tx.clone();
    waits.spawn(async move {
        let why = match child.wait().await {
            Ok(st) => format!("{} exited: {st}", p.name),
            Err(e) => format!("{} exited: {e}", p.name),
        };
        if p.essential {
            let _ = tx.send(why);
        } else {
            eprintln!("odi: {why}");
        }
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn supervise(
    procs: Vec<Proc>,
    site: Option<Site>,
    port: u16,
    web_port: u16,
    health: Option<String>,
    name: &str,
    digest: &str,
    until: Option<Until>,
    live: Option<Arc<Live>>,
) -> Result<()> {
    let (tx, mut exited) = tokio::sync::mpsc::unbounded_channel::<String>();
    let mut waits = tokio::task::JoinSet::new();
    let mut pids = Vec::new();
    for p in procs {
        start_proc(p, &mut waits, &mut pids, &tx)?;
    }
    let (spawn_tx, mut spawn_rx) = tokio::sync::mpsc::unbounded_channel::<Proc>();
    let mut controller = None;
    if let Some(live) = live {
        *live.spawn.lock().expect("lock") = Some(spawn_tx);
        controller = Some(tokio::spawn(control(live, name.to_string())));
    }
    let mut server = None;
    if let Some(site) = site {
        let site = Arc::new(site);
        let tx = tx.clone();
        server = Some(tokio::spawn(async move {
            if let Err(e) = serve(site, port).await {
                let _ = tx.send(e.to_string());
            }
        }));
    }
    if let Some(h) = health {
        tokio::spawn(wait_healthy(web_port, h));
    }
    eprintln!("odi: {name} on http://localhost:{port} ({})", &digest[..19]);

    let until = async {
        match until {
            Some(u) => u.await,
            None => std::future::pending().await,
        }
    };
    tokio::pin!(until);
    let term = terminated();
    tokio::pin!(term);
    let why = loop {
        tokio::select! {
            _ = &mut term => break None,
            w = exited.recv() => break w,
            u = &mut until => break u.err().map(|e| format!("{e:#}")),
            Some(p) = spawn_rx.recv() => {
                let n = p.name.clone();
                if let Err(e) = start_proc(p, &mut waits, &mut pids, &tx) {
                    eprintln!("odi: {n}: {e:#}");
                }
            }
        }
    };
    if let Some(c) = controller {
        c.abort();
    }
    for &pid in &pids {
        terminate(pid);
    }
    if let Some(s) = server {
        s.abort();
    }
    let all_done = async { while waits.join_next().await.is_some() {} };
    if tokio::time::timeout(if cfg!(unix) { Duration::from_secs(10) } else { Duration::ZERO }, all_done).await.is_err()
    {
        waits.abort_all(); // (each child is killed as its task drops it)
        while waits.join_next().await.is_some() {}
    }
    match why {
        Some(w) => Err(anyhow!(w)),
        None => Ok(()),
    }
}

#[cfg(unix)]
async fn terminated() {
    use tokio::signal::unix::{signal, SignalKind};
    match signal(SignalKind::terminate()) {
        Ok(mut t) => {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = t.recv() => {}
            }
        }
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[cfg(not(unix))]
async fn terminated() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(unix)]
fn terminate(pid: u32) {
    // SAFETY: kill(2) with a child's pid and a signal number touches no memory.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
}

#[cfg(not(unix))]
fn terminate(_: u32) {}

pub fn check_platform(p: Option<&Platform>) -> Result<()> {
    let Some(p) = p else { return Ok(()) };
    if (p.os != "any" && p.os != host_os()) || (p.arch != "any" && p.arch != host_arch()) {
        bail!("it's built for {}/{}, this machine is {}/{}", p.os, p.arch, host_os(), host_arch());
    }
    Ok(())
}

/// The program that is each runtime, and how it says its version.
fn runtime_command(name: &str) -> (&str, &'static str) {
    match name {
        "node" => ("node", "--version"),
        "bun" => ("bun", "--version"),
        "deno" => ("deno", "--version"),
        "python" => ("python3", "--version"),
        "ruby" => ("ruby", "--version"),
        "php" => ("php", "--version"),
        "java" => ("java", "-version"),
        "dotnet" => ("dotnet", "--version"),
        other => (other, "--version"),
    }
}

/// The first run of digits and dots in s ("v22.4.0 (x)" -> "22.4.0").
fn first_version(s: &str) -> String {
    let b = s.as_bytes();
    let Some(start) = b.iter().position(u8::is_ascii_digit) else { return String::new() };
    let mut end = start;
    while end < b.len() {
        if b[end].is_ascii_digit() {
            end += 1;
        } else if b[end] == b'.' && b.get(end + 1).is_some_and(u8::is_ascii_digit) {
            end += 2;
        } else {
            break;
        }
    }
    s[start..end].to_string()
}

fn check_runtime(r: &Runtime) -> Result<()> {
    let (prog, flag) = runtime_command(&r.name);
    let path = std::env::var_os("PATH").unwrap_or_default();
    let Some(found) = find_in_path(prog, &path) else {
        bail!("it runs on {} {}, which isn't on this machine (and isn't included in the file)", r.name, r.version);
    };
    if r.version.is_empty() {
        return Ok(());
    }
    let out = std::process::Command::new(found).arg(flag).output().ok();
    let text = out
        .map(|o| format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
        .unwrap_or_default();
    let have = first_version(&text);
    if !version_matches(&have, &r.version) {
        bail!("it runs on {} {}; this machine has {}", r.name, r.version, have);
    }
    Ok(())
}

/// "24.22.0" (exact), "3.13" or "22" (that line), or a range of ">=", ">",
/// "<=", "<", "^", "~" terms (">=22 <25").
pub fn version_matches(have: &str, want: &str) -> bool {
    if have.is_empty() {
        return false;
    }
    let terms: Vec<&str> = want.split_whitespace().collect();
    if terms.len() == 1 && !terms[0].contains(['<', '>', '=', '^', '~']) {
        let w = terms[0].strip_prefix('v').unwrap_or(terms[0]);
        return have == w || have.starts_with(&format!("{w}."));
    }
    let major = |v: &str| v.split('.').next().unwrap_or("").to_string();
    terms.iter().all(|t| {
        let op = t.trim_end_matches(|c: char| c.is_ascii_digit() || matches!(c, '.' | 'x' | 'X' | '*'));
        let v = &t[op.len()..];
        let c = compare_versions(have, v);
        match op {
            ">=" => c >= 0,
            ">" => c > 0,
            "<=" => c <= 0,
            "<" => c < 0,
            "=" | "" => c == 0,
            o if o.starts_with('^') || o.starts_with('~') => c >= 0 && major(have) == major(v),
            _ => false,
        }
    })
}

fn compare_versions(a: &str, b: &str) -> i32 {
    let (a, b): (Vec<&str>, Vec<&str>) = (a.split('.').collect(), b.split('.').collect());
    for i in 0..a.len().max(b.len()) {
        let x: i64 = a.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
        let y: i64 = b.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
        if x != y {
            return if x < y { -1 } else { 1 };
        }
    }
    0
}

fn is_executable(p: &Path) -> bool {
    match fs::metadata(p) {
        Ok(m) if !m.is_dir() => crate::mode_of(&m) & 0o111 != 0 || cfg!(not(unix)),
        _ => false,
    }
}

pub fn find_in_path(name: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    std::env::split_paths(path).map(|d| d.join(name)).find(|p| is_executable(p))
}

/// A relative program name is looked up in runtime/bin, then app/, then
/// PATH (§5.4); a path is relative to the workdir.
fn look_program(name: &str, root: &Path, wd: &Path, path_env: &std::ffi::OsStr) -> PathBuf {
    if name.contains('/') {
        return if Path::new(name).is_absolute() { PathBuf::from(name) } else { wd.join(name) };
    }
    for d in [root.join("runtime").join("bin"), root.join("app")] {
        if is_executable(&d.join(name)) {
            return d.join(name);
        }
    }
    find_in_path(name, path_env).unwrap_or_else(|| PathBuf::from(name))
}

pub fn cache_base() -> Result<PathBuf> {
    if let Some(c) = std::env::var_os("ODI_CACHE").filter(|c| !c.is_empty()) {
        return Ok(PathBuf::from(c));
    }
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Caches"))
    } else if cfg!(windows) {
        env("LocalAppData")
    } else {
        env("XDG_CACHE_HOME").or_else(|| env("HOME").map(|h| h.join(".cache")))
    };
    Ok(base.ok_or_else(|| anyhow!("no cache folder here: set ODI_CACHE"))?.join("odi"))
}

/// Unpacks the file once into a folder named by its digest.
pub fn unpack_cached(rd: &Reader, key: &str) -> Result<PathBuf> {
    let root = cache_base()?.join(key);
    if root.join(".complete").exists() {
        return Ok(root);
    }
    let mut tmp = root.clone().into_os_string();
    tmp.push(format!(".tmp-{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    let _ = fs::remove_dir_all(&tmp);
    if let Err(e) = crate::extract_shared(rd, &tmp, Some(&cache_base()?.join("files"))) {
        let _ = fs::remove_dir_all(&tmp);
        return Err(e);
    }
    fs::write(tmp.join(".complete"), b"")?;
    let _ = fs::remove_dir_all(&root);
    if let Err(e) = fs::rename(&tmp, &root) {
        let _ = fs::remove_dir_all(&tmp);
        return Err(e.into());
    }
    Ok(root)
}

fn free_port() -> Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

// --- static files, and the rest to web -----------------------------------------------

type BoxError = Box<dyn std::error::Error + Send + Sync>;
type Body = UnsyncBoxBody<Bytes, BoxError>;

pub struct Site {
    pub root: PathBuf,
    pub statics: Vec<Static>,
    pub web: Option<Web>,
    pub routes: Option<Arc<RouteHints>>,
}

pub struct Web {
    pub port: u16,
    pub client: Client<HttpConnector, Incoming>,
}

fn text(status: StatusCode, s: &'static str) -> Response<Body> {
    let mut r = Response::new(Full::new(Bytes::from_static(s.as_bytes())).map_err(|e| match e {}).boxed_unsync());
    *r.status_mut() = status;
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    r
}

async fn serve(site: Arc<Site>, port: u16) -> Result<()> {
    let listener = match tokio::net::TcpListener::bind(("::", port)).await {
        Ok(l) => l,
        Err(_) => tokio::net::TcpListener::bind(("0.0.0.0", port)).await.with_context(|| format!("port {port}"))?,
    };
    loop {
        let (stream, peer) = listener.accept().await?;
        let site = site.clone();
        tokio::spawn(async move {
            let svc = hyper::service::service_fn(move |req| handle(site.clone(), req, peer));
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), svc)
                .with_upgrades()
                .await;
        });
    }
}

/// path.Clean for a URL path: absolute, no `.`, `..` or repeated `/`.
fn clean_url_path(p: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for part in p.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            _ => out.push(part),
        }
    }
    format!("/{}", out.join("/"))
}

impl Site {
    /// The static folder a request is for (the longest matching path), and
    /// the file it names, if there is one.
    pub fn static_file(&self, path: &str) -> Option<(PathBuf, bool)> {
        let at_of = |s: &Static| s.at.strip_suffix('/').unwrap_or(&s.at).to_string();
        let mut best: Option<&Static> = None;
        for s in &self.statics {
            let at = at_of(s);
            if (path == at || path.starts_with(&format!("{at}/")) || at.is_empty())
                && best.is_none_or(|b| at.len() > at_of(b).len())
            {
                best = Some(s);
            }
        }
        let s = best?;
        let cleaned = clean_url_path(path);
        let at = at_of(s);
        let rel = cleaned.strip_prefix(&at).unwrap_or(&cleaned);
        let mut p = self.root.join(&s.from);
        for part in rel.split('/').filter(|x| !x.is_empty()) {
            if part.contains(['\\', '\0']) || part == ".." {
                return None;
            }
            p.push(part);
        }
        if p.is_dir() {
            p.push("index.html");
        }
        p.is_file().then_some((p, s.immutable))
    }
}

async fn handle(site: Arc<Site>, req: Request<Incoming>, peer: SocketAddr) -> Result<Response<Body>, Infallible> {
    let decoded = percent_encoding::percent_decode_str(req.uri().path()).decode_utf8().ok().map(|d| d.into_owned());
    // (A route's first request brings all its files in one batch, a page its
    // assets, so neither is fetched file by file as it's read.)
    if let (Some(h), Some(d)) = (&site.routes, decoded.as_deref()) {
        if let Some(i) = h.map.find(d).filter(|i| h.done.lock().expect("lock").insert(*i)) {
            let h = h.clone();
            let _ = tokio::task::spawn_blocking(move || h.fetch(i)).await;
        }
    }
    if req.method() == Method::GET || req.method() == Method::HEAD {
        if let Some((p, immutable)) = decoded.as_deref().and_then(|d| site.static_file(d)) {
            let mut r = Request::new(Empty::<Bytes>::new());
            *r.method_mut() = req.method().clone();
            *r.uri_mut() = req.uri().clone();
            *r.headers_mut() = req.headers().clone();
            let Ok(res) = tower_http::services::ServeFile::new(&p).oneshot(r).await;
            let mut res = res.map(|b| b.map_err(|e| Box::new(e) as BoxError).boxed_unsync());
            if immutable {
                res.headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=31536000, immutable"));
            }
            return Ok(res);
        }
    }
    Ok(match &site.web {
        Some(web) => proxy(web, req, peer).await,
        None => text(StatusCode::NOT_FOUND, "404 page not found\n"),
    })
}

/// The headers that belong to one connection, not to the request (RFC 9110
/// §7.6.1), and any the Connection header names.
fn remove_hop_headers(h: &mut HeaderMap) {
    let named: Vec<String> = h
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(',').map(|s| s.trim().to_ascii_lowercase()))
        .filter(|s| !s.is_empty())
        .collect();
    for n in named {
        h.remove(n.as_str());
    }
    for n in [
        "connection",
        "proxy-connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ] {
        h.remove(n);
    }
}

async fn proxy(web: &Web, mut req: Request<Incoming>, peer: SocketAddr) -> Response<Body> {
    let wants_upgrade = req
        .headers()
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|v| v.split(',').any(|t| t.trim().eq_ignore_ascii_case("upgrade")));
    let upgrade = req.headers().get(header::UPGRADE).cloned().filter(|_| wants_upgrade);
    let client_side = upgrade.as_ref().map(|_| hyper::upgrade::on(&mut req));
    let (mut parts, body) = req.into_parts();
    let pq = parts.uri.path_and_query().map(|x| x.as_str()).unwrap_or("/");
    let Ok(uri) = format!("http://127.0.0.1:{}{}", web.port, pq).parse() else {
        return text(StatusCode::BAD_REQUEST, "bad request\n");
    };
    parts.uri = uri;
    remove_hop_headers(&mut parts.headers);
    if let Some(u) = &upgrade {
        parts.headers.insert(header::CONNECTION, HeaderValue::from_static("Upgrade"));
        parts.headers.insert(header::UPGRADE, u.clone());
    }
    let ip = peer.ip().to_canonical().to_string();
    let xff = match parts.headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        Some(prior) => format!("{prior}, {ip}"),
        None => ip,
    };
    if let Ok(v) = HeaderValue::from_str(&xff) {
        parts.headers.insert("x-forwarded-for", v);
    }
    match web.client.request(Request::from_parts(parts, body)).await {
        Err(e) => {
            // (At most a line a second: a client asking before web is up would
            // otherwise fill the log.)
            static LAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            if LAST.swap(now, Ordering::Relaxed) != now {
                eprintln!("odi: web: {e}");
            }
            text(StatusCode::BAD_GATEWAY, "502 bad gateway\n")
        }
        Ok(mut res) => {
            if res.status() == StatusCode::SWITCHING_PROTOCOLS {
                if let Some(client_side) = client_side {
                    let web_side = hyper::upgrade::on(&mut res);
                    tokio::spawn(async move {
                        if let (Ok(a), Ok(b)) = tokio::join!(client_side, web_side) {
                            let _ = tokio::io::copy_bidirectional(&mut TokioIo::new(a), &mut TokioIo::new(b)).await;
                        }
                    });
                }
            } else {
                remove_hop_headers(res.headers_mut());
            }
            res.map(|b| b.map_err(|e| Box::new(e) as BoxError).boxed_unsync())
        }
    }
}

async fn wait_healthy(port: u16, path: String) {
    let client: Client<HttpConnector, Empty<Bytes>> = Client::builder(TokioExecutor::new()).build_http();
    for _ in 0..120 {
        if let Ok(uri) = format!("http://127.0.0.1:{port}{path}").parse::<hyper::Uri>() {
            if let Ok(res) = client.get(uri).await {
                if res.status().is_success() {
                    eprintln!("odi: web is ready");
                    return;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    eprintln!("odi: web didn't answer {path} within a minute");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        for (have, want, ok) in [
            ("24.22.0", "24.22.0", true),
            ("24.22.1", "24.22.0", false),
            ("22.23.3", "22", true),
            ("3.13.5", "3.13", true),
            ("3.12.1", "3.13", false),
            ("23.1.0", ">=22 <25", true),
            ("25.0.0", ">=22 <25", false),
            ("20.0.0", ">=22", false),
            ("22.4.0", "^22.1", true),
            ("23.0.0", "^22.1", false),
            ("", "22", false),
        ] {
            assert_eq!(version_matches(have, want), ok, "{have} against {want}");
        }
        assert_eq!(first_version("v22.4.0\n"), "22.4.0");
        assert_eq!(first_version("Python 3.13.5"), "3.13.5");
        assert_eq!(first_version("openjdk version \"21.0.2\" 2024"), "21.0.2");
    }

    /// Static files win over web for their paths (the longest match), the
    /// rest goes to web; nothing outside a static folder is served.
    #[test]
    fn static_files() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        fs::create_dir_all(r.join("static/_next/static")).unwrap();
        fs::create_dir_all(r.join("static/public/docs")).unwrap();
        fs::write(r.join("static/_next/static/a.js"), "js").unwrap();
        fs::write(r.join("static/public/logo.svg"), "svg").unwrap();
        fs::write(r.join("static/public/docs/index.html"), "docs").unwrap();
        fs::write(r.join("secret.txt"), "no").unwrap();
        let site = Site {
            root: r.to_path_buf(),
            statics: vec![
                Static { from: "static/_next/static".into(), at: "/_next/static".into(), immutable: true },
                Static { from: "static/public".into(), at: "/".into(), immutable: false },
            ],
            web: None,
            routes: None,
        };
        let (p, imm) = site.static_file("/_next/static/a.js").unwrap();
        assert!(imm && p.ends_with("static/_next/static/a.js"));
        assert!(site.static_file("/logo.svg").unwrap().0.ends_with("public/logo.svg"));
        assert!(site.static_file("/docs").unwrap().0.ends_with("docs/index.html"));
        assert!(site.static_file("/dashboard").is_none());
        assert!(site.static_file("/../secret.txt").is_none());
        assert!(site.static_file("/_next/static/../../../secret.txt").is_none());
    }
}
