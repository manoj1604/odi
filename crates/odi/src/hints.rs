//! Runner hints (SPEC.md §5.11): files under meta/odi/ that let a runner
//! fetch ahead what a program will read. A runner may use them and must run
//! correctly without them, so a hint that's missing, stale or unreadable is
//! ignored.
//!
//! - meta/odi/start.json: what the program read while starting and answering
//!   its first requests, in the order it read it (made by odi profile).
//! - meta/odi/routes.json: for each route of a web app, the files it needs
//!   (made by odi build, from what the framework writes down).

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::Result;
use odi_format::{Kind, Reader, Static};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub const START: &str = "meta/odi/start.json";
pub const ROUTES: &str = "meta/odi/routes.json";

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct StartList {
    pub version: u32,
    pub files: Vec<StartFile>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StartFile {
    pub path: String,
    /// The chunks read, by index in the file (empty: all of them).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chunks: Vec<usize>,
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct RouteMap {
    pub version: u32,
    pub routes: Vec<Route>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Route {
    /// URL segments; `[x]` is one segment, `[...x]` one or more, `[[...x]]`
    /// zero or more (as Next.js and SvelteKit name them).
    pub route: String,
    pub files: Vec<String>,
}

/// A hint from the file, if it's there and readable.
pub fn load<T: DeserializeOwned>(rd: &Reader, path: &str) -> Option<T> {
    rd.entry(path)?;
    serde_json::from_slice(&rd.read_file(path).ok()?).ok()
}

/// Where the stored chunks of these files are in the file, as (offset,
/// length): every chunk of a file, or the ones named.
pub fn ranges<'a>(rd: &Reader, files: impl IntoIterator<Item = (&'a str, &'a [usize])>) -> Vec<(u64, u64)> {
    let mut out = BTreeSet::new();
    for (path, which) in files {
        let Some(e) = rd.entry(path).filter(|e| e.kind == Kind::File) else { continue };
        if which.is_empty() {
            out.extend(e.chunks.iter().map(|c| (c.offset, c.length)));
        } else {
            out.extend(which.iter().filter_map(|&i| e.chunks.get(i)).map(|c| (c.offset, c.length)));
        }
    }
    out.into_iter().collect()
}

impl StartList {
    pub fn ranges(&self, rd: &Reader) -> Vec<(u64, u64)> {
        ranges(rd, self.files.iter().map(|f| (f.path.as_str(), f.chunks.as_slice())))
    }

    /// From reads in the order they happened: (path, chunk index).
    pub fn from_reads(reads: &[(String, usize)], rd: &Reader) -> StartList {
        let mut files: Vec<StartFile> = Vec::new();
        let mut at = std::collections::HashMap::new();
        for (path, chunk) in reads {
            let i = *at.entry(path.clone()).or_insert_with(|| {
                files.push(StartFile { path: path.clone(), chunks: Vec::new() });
                files.len() - 1
            });
            if !files[i].chunks.contains(chunk) {
                files[i].chunks.push(*chunk);
            }
        }
        // (A file read whole is listed without its chunks.)
        for f in &mut files {
            if rd.entry(&f.path).is_some_and(|e| e.chunks.len() == f.chunks.len()) {
                f.chunks.clear();
            }
        }
        StartList { version: 1, files }
    }
}

/// One segment of a route pattern.
enum Seg<'a> {
    Lit(&'a str),
    One,
    OneOrMore,
    ZeroOrMore,
}

fn segments(route: &str) -> Vec<Seg<'_>> {
    route
        .split('/')
        .filter(|s| !s.is_empty() && !(s.starts_with('(') && s.ends_with(')')) && !s.starts_with('@'))
        .map(|s| {
            if s.starts_with("[[...") && s.ends_with("]]") {
                Seg::ZeroOrMore
            } else if s.starts_with("[...") && s.ends_with(']') {
                Seg::OneOrMore
            } else if s.starts_with('[') && s.ends_with(']') {
                Seg::One
            } else {
                Seg::Lit(s)
            }
        })
        .collect()
}

fn matches(pat: &[Seg], path: &[&str]) -> bool {
    match (pat.first(), path.first()) {
        (None, None) => true,
        (None, Some(_)) => false,
        (Some(Seg::ZeroOrMore), _) => true,
        (Some(Seg::OneOrMore), p) => p.is_some(),
        (Some(_), None) => false,
        (Some(Seg::Lit(l)), Some(p)) => l == p && matches(&pat[1..], &path[1..]),
        (Some(Seg::One), Some(_)) => matches(&pat[1..], &path[1..]),
    }
}

impl RouteMap {
    /// The route a URL path is for: of those that match, the one with the
    /// most fixed segments, then the fewest catch-alls.
    pub fn find(&self, url_path: &str) -> Option<usize> {
        let parts: Vec<&str> = url_path.split('/').filter(|s| !s.is_empty()).collect();
        let score = |r: &Route| {
            let s = segments(&r.route);
            let lit = s.iter().filter(|x| matches!(x, Seg::Lit(_))).count();
            let catch = s.iter().filter(|x| matches!(x, Seg::OneOrMore | Seg::ZeroOrMore)).count();
            (lit, usize::MAX - catch)
        };
        self.routes
            .iter()
            .enumerate()
            .filter(|(_, r)| matches(&segments(&r.route), &parts))
            .max_by_key(|(_, r)| score(r))
            .map(|(i, _)| i)
    }
}

/// a/b/../c as a/c, without touching the disk; None if it climbs above.
fn normalize(p: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    Some(out)
}

/// Next.js (app router): each route's files, from the URL map Next writes
/// (`app-paths-manifest.json`) and each route's trace (`.nft.json`: the files
/// it needs). Paths are as in the ODI file (`app/...`); stage is the folder
/// being packed.
pub fn next_routes(stage: &Path) -> Result<Option<RouteMap>> {
    let server = stage.join("app/.next/server");
    let Ok(b) = fs::read(server.join("app-paths-manifest.json")) else { return Ok(None) };
    let manifest: std::collections::BTreeMap<String, String> = serde_json::from_slice(&b)?;
    // Each route's chunks for the browser, as Next lists them.
    #[derive(Deserialize, Default)]
    struct ClientChunks {
        #[serde(default)]
        pages: std::collections::BTreeMap<String, Vec<String>>,
    }
    let client = fs::read(stage.join("app/.next/app-build-manifest.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<ClientChunks>(&b).ok())
        .unwrap_or_default()
        .pages;
    let mut routes = Vec::new();
    for (key, file) in manifest {
        let url = key.strip_suffix("/page").or_else(|| key.strip_suffix("/route")).unwrap_or(&key);
        let url = if url.is_empty() { "/" } else { url };
        let main = server.join(&file);
        let mut wanted = vec![main.clone()];
        let mut nft = main.clone().into_os_string();
        nft.push(".nft.json");
        if let Ok(t) = fs::read(&nft) {
            #[derive(Deserialize)]
            struct Trace {
                #[serde(default)]
                files: Vec<String>,
            }
            if let Ok(t) = serde_json::from_slice::<Trace>(&t) {
                let dir = main.parent().unwrap_or(&server);
                wanted.extend(t.files.iter().map(|f| dir.join(f)));
            }
        }
        let mut seen = HashSet::new();
        let mut files = Vec::new();
        for w in wanted {
            let Some(n) = normalize(&w) else { continue };
            let Ok(rel) = n.strip_prefix(stage) else { continue };
            let Some(rel) = rel.to_str().map(|s| s.replace(std::path::MAIN_SEPARATOR, "/")) else { continue };
            if rel.starts_with("app/") && n.is_file() && seen.insert(rel.clone()) {
                files.push(rel);
            }
        }
        // (And the browser's chunks for the page, which odi serves itself:
        // .next/static/... is static/_next/static/... in the file.)
        for c in client.get(&key).into_iter().flatten() {
            let rel = format!("static/_next/{c}");
            if stage.join(&rel).is_file() && seen.insert(rel.clone()) {
                files.push(rel);
            }
        }
        routes.push(Route { route: url.to_string(), files });
    }
    Ok(Some(RouteMap { version: 1, routes }))
}

/// For each HTML page in the static folders, the page and the files it names
/// (`src="..."`, `href="..."`) that the static folders serve, so a page's
/// first request brings its styles, scripts and images with it. Any web app:
/// a static site, or a framework's static folder.
pub fn static_pages(stage: &Path, statics: &[Static]) -> Vec<Route> {
    // A URL path to the file serving it, as the runner's static handler does
    // (the longest matching `at`).
    let file_for = |url: &str| -> Option<String> {
        let at_of = |s: &Static| s.at.strip_suffix('/').unwrap_or(&s.at).to_string();
        let s = statics
            .iter()
            .filter(|s| {
                let at = at_of(s);
                url == at || url.starts_with(&format!("{at}/")) || at.is_empty()
            })
            .max_by_key(|s| at_of(s).len())?;
        let rel = url.strip_prefix(&at_of(s)).unwrap_or(url).trim_start_matches('/');
        let mut p = normalize(&Path::new(&s.from).join(rel))?;
        if stage.join(&p).is_dir() {
            p.push("index.html");
        }
        let p = p.to_str()?.replace(std::path::MAIN_SEPARATOR, "/");
        stage.join(&p).is_file().then_some(p)
    };
    let mut routes = Vec::new();
    for s in statics {
        let at = s.at.strip_suffix('/').unwrap_or(&s.at).to_string();
        let root = stage.join(&s.from);
        let _ = crate::walk(&root, &mut |p, meta| {
            let is_html = p.extension().is_some_and(|e| e == "html" || e == "htm");
            if !meta.is_file() || !is_html || meta.len() > 2 << 20 {
                return Ok(crate::Step::Continue);
            }
            let Ok(rel) = crate::rel_slash(&root, p) else { return Ok(crate::Step::Continue) };
            // The page's URL, and the folder its relative links start from.
            let url = match rel.strip_suffix("index.html") {
                Some(dir) => format!("{at}/{dir}"),
                None => format!("{at}/{rel}"),
            };
            let base = url[..url.rfind('/').map_or(0, |i| i + 1)].to_string();
            let Ok(html) = fs::read_to_string(p) else { return Ok(crate::Step::Continue) };
            let mut files = vec![];
            let mut seen = HashSet::new();
            let page = format!("{}/{rel}", s.from.trim_end_matches('/'));
            for f in std::iter::once(page).chain(links(&html).filter_map(|l| {
                let l = l.split(['?', '#']).next().unwrap_or("");
                let url = if l.starts_with('/') { l.to_string() } else { format!("{base}{l}") };
                file_for(&normalize(Path::new(&url))?.to_str()?.replace(std::path::MAIN_SEPARATOR, "/"))
            })) {
                if seen.insert(f.clone()) {
                    files.push(f);
                }
            }
            let route = url.trim_end_matches('/');
            routes.push(Route { route: if route.is_empty() { "/".into() } else { route.into() }, files });
            Ok(crate::Step::Continue)
        });
    }
    routes
}

/// The local links in an HTML page: src="..." and href="..." values (either
/// quote), not to another site, not data or fragments.
pub fn links(html: &str) -> impl Iterator<Item = &str> {
    let b = html.as_bytes();
    let mut i = 0;
    std::iter::from_fn(move || {
        while i < b.len() {
            let rest = &html[i..];
            let at = ["src=", "href="].iter().filter_map(|k| rest.find(k).map(|n| (n, k.len()))).min()?;
            i += at.0 + at.1;
            let Some(&q) = b.get(i).filter(|q| **q == b'"' || **q == b'\'') else { continue };
            let end = html[i + 1..].find(q as char)?;
            let v = &html[i + 1..i + 1 + end];
            i += end + 2;
            let foreign = v.contains("://") || v.starts_with("//") || v.starts_with("data:") || v.starts_with('#');
            if !v.is_empty() && !foreign && !v.starts_with("mailto:") && !v.starts_with("javascript:") {
                return Some(v);
            }
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_bring_their_assets() {
        let stage = tempfile::tempdir().unwrap();
        let st = stage.path().join("static");
        fs::create_dir_all(st.join("about")).unwrap();
        fs::create_dir_all(st.join("css")).unwrap();
        fs::write(st.join("css/a.css"), "x").unwrap();
        fs::write(st.join("logo.png"), "x").unwrap();
        fs::write(
            st.join("index.html"),
            r#"<link href="/css/a.css"><img src='logo.png?v=1'><a href="https://x.org/a.png"><img src="data:x"><img src="/missing.png">"#,
        )
        .unwrap();
        fs::write(st.join("about/index.html"), r#"<link href="../css/a.css"><a href="/">home</a>"#).unwrap();
        let statics = [Static { from: "static".into(), at: "/".into(), immutable: false }];
        let mut routes = static_pages(stage.path(), &statics);
        routes.sort_by(|a, b| a.route.cmp(&b.route));
        let got: Vec<(&str, Vec<&str>)> =
            routes.iter().map(|r| (r.route.as_str(), r.files.iter().map(String::as_str).collect())).collect();
        assert_eq!(
            got,
            [
                ("/", vec!["static/index.html", "static/css/a.css", "static/logo.png"]),
                ("/about", vec!["static/about/index.html", "static/css/a.css", "static/index.html"]),
            ]
        );
    }

    #[test]
    fn routes_match_as_next_names_them() {
        let map = RouteMap {
            version: 1,
            routes: [
                "/",
                "/blog",
                "/blog/[slug]",
                "/docs/[...rest]",
                "/shop/[[...all]]",
                "/(group)/about",
                "/api/hello",
            ]
            .iter()
            .map(|r| Route { route: r.to_string(), files: vec![] })
            .collect(),
        };
        let route = |p: &str| map.find(p).map(|i| map.routes[i].route.as_str());
        assert_eq!(route("/"), Some("/"));
        assert_eq!(route("/blog"), Some("/blog"));
        assert_eq!(route("/blog/hello"), Some("/blog/[slug]"));
        assert_eq!(route("/blog/a/b"), None);
        assert_eq!(route("/docs"), None);
        assert_eq!(route("/docs/a/b/c"), Some("/docs/[...rest]"));
        assert_eq!(route("/shop"), Some("/shop/[[...all]]"));
        assert_eq!(route("/shop/x/y"), Some("/shop/[[...all]]"));
        assert_eq!(route("/about"), Some("/(group)/about"));
        assert_eq!(route("/api/hello"), Some("/api/hello"));
        assert_eq!(route("/nope"), None);
    }
}
