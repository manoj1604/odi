//! odi.json: every project's build settings (docs/odi-json.md is the
//! specification; schema/odi.schema.json its JSON Schema). `name` and `type`
//! are required; any key not in the specification makes the file invalid.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde_json::{Map, Value};

pub const SCHEMA: &str = "https://raw.githubusercontent.com/manoj1604/odi/main/schema/odi.schema.json";
const SPEC: &str = "https://github.com/manoj1604/odi/blob/main/docs/odi-json.md";

/// What kind of project: odi.json's `type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Go,
    Node,
    Nextjs,
    Python,
    Static,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "go" => Kind::Go,
            "node" => Kind::Node,
            "nextjs" => Kind::Nextjs,
            "python" => Kind::Python,
            "static" => Kind::Static,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Go => "go",
            Kind::Node => "node",
            Kind::Nextjs => "nextjs",
            Kind::Python => "python",
            Kind::Static => "static",
        }
    }

    /// The runtime this kind of project runs on, if any.
    pub fn runtime(self) -> Option<&'static str> {
        match self {
            Kind::Node | Kind::Nextjs => Some("node"),
            Kind::Python => Some("python"),
            Kind::Go | Kind::Static => None,
        }
    }
}

#[derive(Debug)]
pub struct OdiJson {
    pub name: String,
    pub kind: Kind,
    pub version: Option<String>,
    pub repo: Option<String>,
    /// "node 22" -> ("node", "22"); "node" -> ("node", "").
    pub runtime: Option<(String, String)>,
    pub include_runtime: bool,
    /// Some(""): build nothing.
    pub build: Option<String>,
    pub start: Option<String>,
    pub port: Option<i64>,
    pub r#static: Option<String>,
    /// KEY -> a default value, or "required" / "optional".
    pub env: BTreeMap<String, String>,
    /// Raw manifest fields.
    pub manifest: Map<String, Value>,
}

impl OdiJson {
    /// Just a name and a type, the rest left out.
    pub fn new(name: &str, kind: Kind) -> OdiJson {
        OdiJson {
            name: name.into(),
            kind,
            version: None,
            repo: None,
            runtime: None,
            include_runtime: false,
            build: None,
            start: None,
            port: None,
            r#static: None,
            env: BTreeMap::new(),
            manifest: Map::new(),
        }
    }
}

const KEYS: &[&str] = &[
    "$schema",
    "name",
    "type",
    "version",
    "repo",
    "runtime",
    "include_runtime",
    "build",
    "start",
    "port",
    "static",
    "env",
    "manifest",
];

impl OdiJson {
    /// The project's odi.json; a project without one isn't one odi builds.
    pub fn load(dir: &Path) -> Result<OdiJson> {
        let path = dir.join("odi.json");
        let b = std::fs::read(&path)
            .map_err(|_| anyhow!("no odi.json in {}: odi init writes one for this project ({SPEC})", dir.display()))?;
        Self::parse(&b)
    }

    pub fn parse(b: &[u8]) -> Result<OdiJson> {
        let bad = |what: String| anyhow!("odi.json: {what} ({SPEC})");
        let Value::Object(o) = serde_json::from_slice(b).context("odi.json isn't valid JSON")? else {
            return Err(bad("an object, { ... }".into()));
        };
        if let Some(k) = o.keys().find(|k| !KEYS.contains(&k.as_str())) {
            let near = KEYS.iter().find(|w| crate::cli::distance(w, k) <= 2);
            let hint = near.map(|w| format!(" (did you mean {w:?}?)")).unwrap_or_default();
            return Err(bad(format!("no key {k:?}{hint}")));
        }
        let text = |k: &str, empty_ok: bool| -> Result<Option<String>> {
            match o.get(k) {
                None => Ok(None),
                Some(Value::String(s)) if empty_ok || !s.trim().is_empty() => Ok(Some(s.trim().to_string())),
                Some(Value::String(_)) => Err(bad(format!("{k:?} is empty"))),
                Some(_) => Err(bad(format!("{k:?} is text, in quotes"))),
            }
        };
        let name = text("name", false)?.ok_or_else(|| bad("\"name\" is required: \"name\": \"shop\"".into()))?;
        if !valid_name(&name) {
            return Err(bad(format!("\"name\": {name:?}: 1 to 64 of a-z 0-9 . _ -, starting with a letter or digit")));
        }
        let t = text("type", false)?
            .ok_or_else(|| bad("\"type\" is required: go, node, nextjs, python or static".into()))?;
        let kind =
            Kind::parse(&t).ok_or_else(|| bad(format!("\"type\": {t:?}: go, node, nextjs, python or static")))?;
        let repo = text("repo", false)?;
        if let Some(r) = &repo {
            if !r.split_once('/').is_some_and(|(a, b)| valid_name(a) && valid_name(b)) {
                return Err(bad(format!("\"repo\": {r:?}: namespace/name, such as acme/shop")));
            }
        }
        let runtime = match text("runtime", false)? {
            None => None,
            Some(r) => {
                let mut w = r.split_whitespace();
                let n = w.next().unwrap_or_default().to_string();
                let v = w.next().unwrap_or_default().trim_start_matches('v').to_string();
                let version_ok = v.is_empty()
                    || (v.split('.').count() <= 3
                        && v.split('.').all(|x| !x.is_empty() && x.bytes().all(|c| c.is_ascii_digit())));
                if w.next().is_some() || !version_ok {
                    return Err(bad(format!("\"runtime\": {r:?}: a name and a version, \"node 22\"")));
                }
                match kind.runtime() {
                    Some(want) if want == n => {}
                    Some(want) => return Err(bad(format!("\"runtime\": {n}, but a {t} project runs on {want}"))),
                    None => return Err(bad(format!("\"runtime\": a {t} project has no runtime"))),
                }
                Some((n, v))
            }
        };
        let include_runtime = match o.get("include_runtime") {
            None => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(bad("\"include_runtime\" is true or false".into())),
        };
        if include_runtime {
            match kind.runtime() {
                None => return Err(bad(format!("\"include_runtime\": a {t} project has no runtime"))),
                Some("node") => {}
                Some(r) => {
                    return Err(bad(format!("\"include_runtime\": odi can put Node.js in a file for now, not {r}")))
                }
            }
        }
        let port = match o.get("port") {
            None => None,
            Some(Value::Number(n)) if n.as_i64().is_some_and(|p| (1..=65535).contains(&p)) => n.as_i64(),
            Some(_) => return Err(bad("\"port\" is a number, 1 to 65535".into())),
        };
        let r#static = text("static", false)?;
        if r#static.is_some() && kind != Kind::Static {
            return Err(bad(format!("\"static\" is for static projects, not {t}")));
        }
        if kind == Kind::Static && o.contains_key("start") {
            return Err(bad("\"start\": a static project has nothing to start (odi serves its folder)".into()));
        }
        let mut env = BTreeMap::new();
        match o.get("env") {
            None => {}
            Some(Value::Object(e)) => {
                for (k, v) in e {
                    let v = match v {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        _ => return Err(bad(format!("env {k:?}: a value, \"required\" or \"optional\""))),
                    };
                    env.insert(k.clone(), v);
                }
            }
            Some(_) => return Err(bad("\"env\" is an object: {\"KEY\": \"value\"}".into())),
        }
        let manifest = match o.get("manifest") {
            None => Map::new(),
            Some(Value::Object(m)) => m.clone(),
            Some(_) => return Err(bad("\"manifest\" is an object of manifest fields".into())),
        };
        Ok(OdiJson {
            name,
            kind,
            version: text("version", false)?,
            repo,
            runtime,
            include_runtime,
            build: text("build", true)?,
            start: text("start", false)?,
            port,
            r#static,
            env,
            manifest,
        })
    }
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && s.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-'))
}

/// A command as a process runs it: its words when it's plain, else through
/// sh (pipes, variables, &&).
pub fn command(cmd: &str) -> Vec<String> {
    if cmd.contains(['&', '|', ';', '$', '>', '<', '`', '"', '\'', '*', '(']) {
        vec!["sh".into(), "-c".into(), cmd.into()]
    } else {
        cmd.split_whitespace().map(String::from).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec() {
        let j = OdiJson::parse(
            br#"{"$schema": "x", "name": "shop", "type": "nextjs", "repo": "acme/shop", "runtime": "node 22",
                 "include_runtime": true, "build": "npm run build", "start": "node server.js", "port": 3000,
                 "env": {"NODE_ENV": "production", "DATABASE_URL": "required"}, "version": "1.2.0",
                 "manifest": {"writable": ["app/tmp"]}}"#,
        )
        .unwrap();
        assert_eq!(j.kind, Kind::Nextjs);
        assert_eq!(j.runtime, Some(("node".into(), "22".into())));
        assert!(j.include_runtime);
        assert_eq!(j.port, Some(3000));
        assert_eq!(j.env["DATABASE_URL"], "required");
        assert_eq!(j.repo.as_deref(), Some("acme/shop"));
        assert!(j.manifest.contains_key("writable"));
        assert_eq!(OdiJson::parse(br#"{"name": "a", "type": "go", "build": ""}"#).unwrap().build.as_deref(), Some(""));

        let err = |b: &[u8]| OdiJson::parse(b).unwrap_err().to_string();
        assert!(err(br#"{"type": "go"}"#).contains("\"name\" is required"));
        assert!(err(br#"{"name": "a"}"#).contains("\"type\" is required"));
        assert!(err(br#"{"name": "a", "type": "rust"}"#).contains("go, node"));
        assert!(err(br#"{"name": "Shop", "type": "go"}"#).contains("a-z"));
        assert!(
            err(br#"{"name": "a", "type": "go", "includ_runtime": true}"#).contains("did you mean \"include_runtime\"")
        );
        assert!(err(br#"{"name": "a", "type": "go", "runtime": "node 22"}"#).contains("no runtime"));
        assert!(err(br#"{"name": "a", "type": "node", "runtime": "python 3"}"#).contains("runs on node"));
        assert!(err(br#"{"name": "a", "type": "python", "include_runtime": true}"#).contains("for now"));
        assert!(err(br#"{"name": "a", "type": "node", "port": "80"}"#).contains("a number"));
        assert!(err(br#"{"name": "a", "type": "node", "static": "dist"}"#).contains("static projects"));
        assert!(err(br#"{"name": "a", "type": "go", "repo": "shop"}"#).contains("namespace/name"));
        assert!(err(br#"{"name": "a", "type": "static", "start": "x"}"#).contains("nothing to start"));
        assert_eq!(command("node server.js"), vec!["node", "server.js"]);
        assert_eq!(command("npm run migrate && node server.js")[0], "sh");
    }
}
