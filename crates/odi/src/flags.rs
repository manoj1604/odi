//! Flags as Go's flag package reads them (`-o x`, `--o=x`, `-port 8080`,
//! `--port 8080`), but anywhere among the positional arguments:
//! `odi run app.odi --port 9`. `-h`/`--help` prints the command's help.

use std::collections::HashMap;

use anyhow::{bail, Result};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Value,
    /// Repeatable: every value is kept.
    Multi,
}

pub struct Spec {
    pub name: &'static str,
    pub kind: Kind,
    pub help: &'static str,
}

pub const fn flag(name: &'static str, kind: Kind, help: &'static str) -> Spec {
    Spec { name, kind, help }
}

pub struct Flags {
    vals: HashMap<&'static str, Vec<String>>,
}

impl Flags {
    pub fn str(&self, name: &str) -> Option<&str> {
        self.vals.get(name).and_then(|v| v.last()).map(String::as_str)
    }

    pub fn bool(&self, name: &str) -> bool {
        self.str(name) == Some("true")
    }

    pub fn all(&self, name: &str) -> &[String] {
        self.vals.get(name).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn int(&self, name: &str) -> Result<Option<i64>> {
        match self.str(name) {
            None => Ok(None),
            Some(s) => match s.parse() {
                Ok(n) => Ok(Some(n)),
                Err(_) => bail!("--{name}: {s:?} isn't a number"),
            },
        }
    }
}

/// The command's help (cli.rs), found by the usage line's command name.
fn help(usage: &str, specs: &[Spec]) -> String {
    match usage.split_whitespace().nth(1).and_then(crate::cli::find) {
        Some(c) => crate::cli::command_help(c, specs),
        None => {
            let mut s = format!("usage: {usage}\n");
            for sp in specs {
                s.push_str(&format!("  --{}  {}\n", sp.name, sp.help));
            }
            s
        }
    }
}

/// How the command's help is asked for, for errors.
fn see(usage: &str) -> String {
    match usage.split_whitespace().nth(1) {
        Some(cmd) => format!("(odi {cmd} --help lists them)"),
        None => String::new(),
    }
}

/// Parses flags that may come before, between or after the positional
/// arguments; everything after `--` is positional.
pub fn parse(specs: &[Spec], args: &[String], usage: &str) -> Result<(Flags, Vec<String>)> {
    let mut vals: HashMap<&'static str, Vec<String>> = HashMap::new();
    let mut pos = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--" {
            pos.extend(it.by_ref().cloned());
            break;
        }
        if a.len() < 2 || !a.starts_with('-') {
            pos.push(a.clone());
            continue;
        }
        let body = a.strip_prefix("--").unwrap_or(&a[1..]);
        let (name, inline) = match body.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (body, None),
        };
        if name == "h" || name == "help" {
            print!("{}", help(usage, specs));
            std::process::exit(0);
        }
        let Some(sp) = specs.iter().find(|s| s.name == name) else {
            let dashes = if name.len() == 1 { "-" } else { "--" };
            bail!("unknown flag {dashes}{name} {}", see(usage));
        };
        let v = match (sp.kind, inline) {
            (Kind::Bool, None) => "true".to_string(),
            (Kind::Bool, Some(v)) => match v.as_str() {
                "1" | "t" | "T" | "true" | "TRUE" | "True" => "true".into(),
                "0" | "f" | "F" | "false" | "FALSE" | "False" => "false".into(),
                _ => bail!("--{name}: {v:?} isn't true or false"),
            },
            (_, Some(v)) => v,
            (_, None) => match it.next() {
                Some(v) => v.clone(),
                None => bail!("--{name} needs a value {}", see(usage)),
            },
        };
        let slot = vals.entry(sp.name).or_default();
        if sp.kind != Kind::Multi {
            slot.clear();
        }
        slot.push(v);
    }
    Ok((Flags { vals }, pos))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anywhere() {
        let specs = [flag("o", Kind::Value, ""), flag("no-build", Kind::Bool, ""), flag("e", Kind::Multi, "")];
        let args: Vec<String> = ["a", "-o", "x.odi", "b", "--no-build", "-e", "K=1", "--e=L=2", "--", "-o"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (f, pos) = parse(&specs, &args, "").unwrap();
        assert_eq!(pos, ["a", "b", "-o"]);
        assert_eq!(f.str("o"), Some("x.odi"));
        assert!(f.bool("no-build"));
        assert_eq!(f.all("e"), ["K=1", "L=2"]);
        assert!(parse(&specs, &["-x".to_string()], "").is_err());
        assert!(parse(&specs, &["-o".to_string()], "").is_err());
    }
}
