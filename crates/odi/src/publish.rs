//! Working with a hub: publishing versions (odi push), what a repository
//! has (odi versions), moving tags without pushing again (odi tag),
//! comparing two builds (odi diff), and finding repositories (odi search,
//! odi repos). Versions are tags (`1.4.2`); a published version doesn't
//! change, while other tags (`latest`, `staging`) move.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use odi_format::{digest_string, Kind, Reader};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::flags::{self, flag, Kind::*};
use crate::hub::{
    is_status, new_client, nullable, only_odi_file, parse_ref, path_escape, platform_of, push_file, Client, Ref,
    TagPlatform,
};
use crate::human;
use crate::versions::{newest, Range, Version};

/// A tag, as a hub lists it.
#[derive(Deserialize, Clone, Default)]
pub struct Tag {
    #[serde(default)]
    pub tag: String,
    #[serde(default)]
    pub digest: String,
    #[serde(default)]
    pub updated: i64,
    #[serde(default, deserialize_with = "nullable")]
    pub platforms: Vec<TagPlatform>,
}

/// A repository's tags (none when it doesn't exist yet).
fn tags_of(c: &Client, r: &Ref) -> Result<Vec<Tag>> {
    match c.json("GET", &format!("/v1/repos/{}/tags", r.repo()), None) {
        Ok(t) => Ok(t),
        Err(e) if is_status(&e, 404) => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

fn tag_of(c: &Client, r: &Ref, tag: &str) -> Result<Option<Tag>> {
    match c.json::<Tag>("GET", &format!("/v1/repos/{}/tags/{}", r.repo(), path_escape(tag)), None) {
        Ok(t) => Ok(Some(t)),
        Err(e) if is_status(&e, 404) => Ok(None),
        Err(e) => Err(e),
    }
}

/// The newest version in r's range (`1.4`, `^1`), from the hub's tags.
pub fn pick_version(c: &Client, r: &Ref) -> Result<String> {
    let range = Range::parse(&r.tag).ok_or_else(|| anyhow!("{:?} isn't a version range", r.tag))?;
    let tags: Vec<Tag> = c.json("GET", &format!("/v1/repos/{}/tags", r.repo()), None).map_err(|e| {
        if is_status(&e, 404) {
            anyhow!("{}: no such repository", r.repo())
        } else {
            e
        }
    })?;
    range.pick(tags.iter().map(|t| t.tag.as_str())).map(String::from).ok_or_else(|| {
        let have: Vec<&str> = tags.iter().map(|t| t.tag.as_str()).filter(|t| Version::parse(t).is_some()).collect();
        anyhow!(
            "{}:{}: no version in that range ({})",
            r.repo(),
            r.tag,
            if have.is_empty() { "it has none yet".to_string() } else { format!("it has {}", have.join(", ")) }
        )
    })
}

/// Whether the name string gave a tag (`acme/web:1.4`), not just a name.
fn names_tag(s: &str) -> bool {
    s[s.rfind('/').unwrap_or(0)..].contains([':', '@'])
}

/// The repository odi.json names (`"repo": "acme/web"`), if any.
pub fn project_repo(dir: &Path) -> Option<String> {
    let b = fs::read(dir.join("odi.json")).ok()?;
    let v: Value = serde_json::from_slice(&b).ok()?;
    v.get("repo")?.as_str().map(String::from)
}

fn same_platforms(a: &[TagPlatform], b: &[TagPlatform]) -> bool {
    let set = |l: &[TagPlatform]| -> BTreeSet<(String, String)> {
        l.iter().map(|p| (p.platform.clone(), p.digest.trim_start_matches("sha256:").to_string())).collect()
    };
    set(a) == set(b)
}

/// Points tag at these builds. A version, once published, stays: moving it
/// to other builds needs force (adding a platform it lacks is fine with
/// add). Other tags move. Returns false when it already pointed there.
fn set_tag(c: &Client, r: &Ref, tag: &str, list: &[TagPlatform], add: bool, force: bool) -> Result<bool> {
    let path = format!("/v1/repos/{}/tags/{}", r.repo(), path_escape(tag));
    let body = serde_json::to_vec(&json!({ "platforms": list, "add": add }))?;
    let is_version = Version::parse(tag).is_some();
    let cur = tag_of(c, r, tag)?;
    let mut if_match: Option<String> = None;
    match &cur {
        None if is_version => if_match = Some(String::new()), // (it must still not exist)
        Some(t) if is_version && !force => {
            let held: HashMap<&str, &str> =
                t.platforms.iter().map(|p| (p.platform.as_str(), p.digest.trim_start_matches("sha256:"))).collect();
            let clash = list
                .iter()
                .find(|p| held.get(p.platform.as_str()).is_some_and(|d| *d != p.digest.trim_start_matches("sha256:")));
            if let Some(p) = clash {
                bail!(
                    "{}:{tag} is already published, with another build for {} (sha256:{}): a version doesn't change. \
                     Push a new version (--bump patch), or --force to move it",
                    r.repo(),
                    p.platform,
                    &held[p.platform.as_str()][..12.min(held[p.platform.as_str()].len())]
                );
            }
            if same_platforms(&t.platforms, list) || list.iter().all(|p| held.contains_key(p.platform.as_str())) {
                return Ok(false);
            }
            if !add {
                bail!(
                    "{}:{tag} is already published for {}: add this platform to it with --add",
                    r.repo(),
                    t.platforms.iter().map(|p| p.platform.as_str()).collect::<Vec<_>>().join(", ")
                );
            }
            if_match = Some(t.digest.clone());
        }
        Some(t) if same_platforms(&t.platforms, list) => return Ok(false),
        _ => {}
    }
    let headers: Vec<(&str, &str)> = if_match.as_deref().map(|m| vec![("If-Match", m)]).unwrap_or_default();
    match c.call_with("PUT", &path, Some(&body), Some("application/json"), &headers) {
        Ok(_) => Ok(true),
        Err(e) if is_status(&e, 412) => Err(e.context(format!(
            "{}:{tag} was published by someone else just now: look (odi versions), then push again",
            r.repo()
        ))),
        Err(e) => Err(e),
    }
}

pub fn cmd_push(args: &[String]) -> Result<()> {
    const U: &str = "odi push [NAMESPACE/NAME[:TAG]] [FILE...] [--bump patch|minor|major] [--tag TAG]... [--no-latest] [--add] [--public] [--force]";
    let specs = [
        flag("bump", Value, "publish the next version after the newest one: patch, minor or major"),
        flag("tag", Multi, "another tag to point at it, such as staging (repeat)"),
        flag("no-latest", Bool, "don't move latest to it"),
        flag("add", Bool, "add to the platforms the version or tag already has (one build per platform)"),
        flag("public", Bool, "make the repository public"),
        flag("force", Bool, "move a version that's already published to these builds"),
    ];
    let (f, pos) = flags::parse(&specs, args, U)?;
    // The name comes first, unless the first is a file (then it's odi.json's).
    let first_is_file = pos.first().is_some_and(|p| p.ends_with(".odi") || Path::new(p).is_file());
    let (name, files) = match (pos.first(), first_is_file) {
        (Some(n), false) => (n.clone(), pos[1..].to_vec()),
        _ => (
            project_repo(Path::new(".")).ok_or_else(|| {
                anyhow!(
                    "name the repository: odi push NAMESPACE/NAME (or put \"repo\": \"NAMESPACE/NAME\" in odi.json)"
                )
            })?,
            pos.clone(),
        ),
    };
    let r = parse_ref(&name)?;
    if !r.digest.is_empty() {
        bail!("push to a name or a version, not a digest: a digest comes from the file");
    }
    let files = if files.is_empty() { vec![only_odi_file()?] } else { files };
    let c = new_client(&r);
    if !c.signed_in() {
        bail!("sign in first: odi login");
    }

    // Which tags: the one named; or the next version (--bump); or the
    // files' own version, when it's one. Then latest, for a release.
    let mut tags: Vec<String> = Vec::new();
    if names_tag(&name) {
        if f.str("bump").is_some() {
            bail!("--bump picks the version: give the name without one");
        }
        tags.push(r.tag.clone());
    } else if let Some(part) = f.str("bump") {
        let existing = tags_of(&c, &r)?;
        let base = newest(existing.iter().map(|t| t.tag.as_str())).unwrap_or(Version {
            major: 0,
            minor: 0,
            patch: 0,
            pre: String::new(),
            v: false,
        });
        let next = base.bump(part).ok_or_else(|| anyhow!("--bump {part}: patch, minor or major"))?;
        tags.push(next.to_string());
    } else {
        let mut versions = BTreeSet::new();
        for p in &files {
            versions.insert(Reader::open(p).with_context(|| p.clone())?.manifest.version);
        }
        if versions.len() == 1 {
            let v = versions.into_iter().next().expect("one");
            if Version::parse(&v).is_some() {
                tags.push(v);
            }
        }
    }
    let release = tags.first().and_then(|t| Version::parse(t)).is_some_and(|v| v.is_release());
    if !f.bool("no-latest") && (tags.is_empty() || release) {
        tags.push("latest".into());
    }
    for t in f.all("tag") {
        if !tags.contains(t) {
            tags.push(t.clone());
        }
    }
    if tags.is_empty() {
        bail!("nothing to tag: name a version (NAME:1.0.0), --bump, or --tag");
    }

    // The builds, then the tags (a version first: if it's taken, nothing moves).
    let base = format!("/v1/repos/{}", r.repo());
    let mut list: Vec<TagPlatform> = Vec::new();
    for path in &files {
        let p = push_file(&c, &base, path).with_context(|| path.clone())?;
        if let Some(other) = list.iter().find(|o| o.platform == p.platform) {
            bail!("two files here are for {} ({} and {path})", p.platform, other.digest);
        }
        list.push(p);
    }
    let mut moved = Vec::new();
    for t in &tags {
        let changed = set_tag(&c, &r, t, &list, f.bool("add"), f.bool("force"))?;
        moved.push(if changed { t.clone() } else { format!("{t} (already)") });
    }
    if f.bool("public") {
        c.send_json("PUT", &base, Some(&json!({"public": true})))?;
    }
    println!("{}  {}", r.repo(), moved.join(", "));
    Ok(())
}

/// What a repository is, as a hub shows it.
#[derive(Deserialize, Default)]
#[serde(default)]
struct RepoInfo {
    public: bool,
    pulls: u64,
    language: String,
    #[serde(deserialize_with = "nullable")]
    tags: Vec<Tag>,
    #[serde(deserialize_with = "nullable")]
    digests: Vec<BuildInfo>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct BuildInfo {
    digest: String,
    size: u64,
    created: i64,
}

fn when(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

fn short(d: &str) -> String {
    let d = d.trim_start_matches("sha256:");
    format!("sha256:{}", &d[..12.min(d.len())])
}

pub fn cmd_versions(args: &[String]) -> Result<()> {
    const U: &str = "odi versions NAMESPACE/NAME [--all]";
    let specs = [flag("all", Bool, "also list builds no tag points at")];
    let (f, pos) = flags::parse(&specs, args, U)?;
    let name = match pos.as_slice() {
        [n] => n.clone(),
        [] => project_repo(Path::new(".")).ok_or_else(|| anyhow!("usage: {U}"))?,
        _ => bail!("usage: {U}"),
    };
    let r = parse_ref(&name)?;
    let c = new_client(&r);
    let info: RepoInfo = c.json("GET", &format!("/v1/repos/{}", r.repo()), None)?;
    let sizes: HashMap<String, &BuildInfo> =
        info.digests.iter().map(|b| (b.digest.trim_start_matches("sha256:").to_string(), b)).collect();
    let key = |t: &Tag| -> Vec<String> {
        let mut k: Vec<String> =
            t.platforms.iter().map(|p| p.digest.trim_start_matches("sha256:").to_string()).collect();
        k.sort();
        k
    };
    let mut versions: Vec<(Version, &Tag)> =
        info.tags.iter().filter_map(|t| Version::parse(&t.tag).map(|v| (v, t))).collect();
    versions.sort_by(|a, b| b.0.cmp(&a.0));
    let others: Vec<&Tag> = info.tags.iter().filter(|t| Version::parse(&t.tag).is_none()).collect();

    let lang = if info.language.is_empty() { String::new() } else { format!(" · {}", info.language) };
    println!(
        "{}  {} · {} versions · {} builds · {} pulls{lang}",
        r.repo(),
        if info.public { "public" } else { "private" },
        versions.len(),
        info.digests.len(),
        info.pulls
    );
    if info.tags.is_empty() {
        println!("\nnothing published yet: odi push {} FILE", r.repo());
        return Ok(());
    }
    let size_of = |t: &Tag| {
        t.platforms
            .first()
            .and_then(|p| sizes.get(p.digest.trim_start_matches("sha256:")))
            .map(|b| human(b.size))
            .unwrap_or_default()
    };
    let plats = |t: &Tag| t.platforms.iter().map(|p| p.platform.as_str()).collect::<Vec<_>>().join(" ");
    let mut rows: Vec<[String; 5]> = Vec::new();
    let mut shown: BTreeSet<&str> = BTreeSet::new();
    // Each other tag once: by the newest release with the same builds (a
    // pre-release only when no release has them).
    let home = |o: &Tag| -> Option<usize> {
        let same = |i: &usize| key(versions[*i].1) == key(o);
        (0..versions.len())
            .filter(|i| versions[*i].0.is_release())
            .find(same)
            .or_else(|| (0..versions.len()).find(same))
    };
    let homes: Vec<Option<usize>> = others.iter().map(|o| home(o)).collect();
    for (i, (v, t)) in versions.iter().enumerate() {
        let also: Vec<&str> =
            others.iter().zip(&homes).filter(|(_, h)| **h == Some(i)).map(|(o, _)| o.tag.as_str()).collect();
        shown.extend(also.iter().copied());
        rows.push([v.to_string(), also.join(" "), plats(t), size_of(t), when(t.updated)]);
    }
    for o in others.iter().filter(|o| !shown.contains(o.tag.as_str())) {
        rows.push([
            "-".into(),
            o.tag.clone(),
            plats(o),
            size_of(o),
            format!("{} {}", when(o.updated), short(o.platforms.first().map(|p| p.digest.as_str()).unwrap_or(""))),
        ]);
    }
    let head = ["VERSION", "TAGS", "PLATFORMS", "SIZE", "UPDATED"];
    let w: Vec<usize> =
        (0..5).map(|i| rows.iter().map(|r| r[i].len()).chain([head[i].len()]).max().unwrap_or(0)).collect();
    println!();
    for row in std::iter::once(head.map(String::from)).chain(rows) {
        let line: Vec<String> = row.iter().enumerate().map(|(i, s)| format!("{s:<w$}", w = w[i])).collect();
        println!("{}", line.join("  ").trim_end());
    }
    let tagged: BTreeSet<String> = info.tags.iter().flat_map(key).collect();
    let loose: Vec<&BuildInfo> =
        info.digests.iter().filter(|b| !tagged.contains(b.digest.trim_start_matches("sha256:"))).collect();
    if !loose.is_empty() {
        if f.bool("all") {
            println!("\nbuilds no tag points at:");
            for b in &loose {
                println!("  {}  {}  {}", short(&b.digest), human(b.size), when(b.created));
            }
        } else {
            println!(
                "\n{} more builds no tag points at (--all lists them; odi rm {}@sha256:… frees their space)",
                loose.len(),
                r.repo()
            );
        }
    }
    Ok(())
}

/// The builds a name points at, every platform: a tag's (or a range's),
/// or one build by digest.
fn builds_of(c: &Client, r: &Ref) -> Result<(String, Vec<TagPlatform>)> {
    if !r.digest.is_empty() {
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct B {
            manifest: Value,
        }
        let b: B = c.json("GET", &format!("/v1/repos/{}/digests/{}", r.repo(), r.digest), None)?;
        let m: odi_format::Manifest = serde_json::from_value(b.manifest).unwrap_or_default();
        return Ok((short(&r.digest), vec![TagPlatform { platform: platform_of(&m), digest: r.digest.clone() }]));
    }
    let tag = match tag_of(c, r, &r.tag)? {
        Some(t) => return Ok((r.tag.clone(), t.platforms)),
        None if Range::parse(&r.tag).is_some() => pick_version(c, r)?,
        None => bail!("{}:{}: no such tag (odi versions {} lists them)", r.repo(), r.tag, r.repo()),
    };
    let t = tag_of(c, r, &tag)?.ok_or_else(|| anyhow!("{}:{tag} went away", r.repo()))?;
    Ok((tag, t.platforms))
}

pub fn cmd_tag(args: &[String]) -> Result<()> {
    const U: &str = "odi tag NAMESPACE/NAME:FROM TAG... [--force]";
    let specs = [flag("force", Bool, "move a version that's already published")];
    let (f, pos) = flags::parse(&specs, args, U)?;
    if pos.len() < 2 {
        bail!("usage: {U}");
    }
    let r = parse_ref(&pos[0])?;
    if !names_tag(&pos[0]) {
        bail!("name what to point at: {}:VERSION, {}:TAG or {}@sha256:DIGEST", r.repo(), r.repo(), r.repo());
    }
    let c = new_client(&r);
    if !c.signed_in() {
        bail!("sign in first: odi login");
    }
    let (from, list) = builds_of(&c, &r)?;
    for t in &pos[1..] {
        let moved = set_tag(&c, &r, t, &list, false, f.bool("force"))?;
        let what = list.iter().map(|p| format!("{} {}", p.platform, short(&p.digest))).collect::<Vec<_>>().join(", ");
        println!("{}:{t} -> {from}{}  ({what})", r.repo(), if moved { "" } else { " (already)" });
    }
    Ok(())
}

pub fn cmd_diff(args: &[String]) -> Result<()> {
    const U: &str = "odi diff OLD NEW [--all]   (each an ODI file or a name in a registry)";
    let specs = [flag("all", Bool, "list every changed file, not just the first 40")];
    let (f, pos) = flags::parse(&specs, args, U)?;
    let [a, b] = pos.as_slice() else { bail!("usage: {U}") };
    let (ra, _) = crate::hub::open_any(a)?;
    let (rb, _) = crate::hub::open_any(b)?;
    println!("{a} -> {b}");
    if ra.digest == rb.digest {
        println!("  the same build ({})", digest_string(&ra.digest));
        return Ok(());
    }

    // The manifest: what changes how it runs.
    let (ma, mb) = (&ra.manifest, &rb.manifest);
    let mut said = Vec::new();
    let mut field = |what: &str, x: String, y: String| {
        if x != y {
            said.push(format!(
                "  {what:<10}{} -> {}",
                if x.is_empty() { "(none)" } else { &x },
                if y.is_empty() { "(none)" } else { &y }
            ));
        }
    };
    field("version", ma.version.clone(), mb.version.clone());
    field("platform", platform_of(ma), platform_of(mb));
    let rt =
        |m: &odi_format::Manifest| m.runtime.as_ref().map(|r| format!("{} {}", r.name, r.version)).unwrap_or_default();
    field("runtime", rt(ma), rt(mb));
    let procs = |m: &odi_format::Manifest| {
        m.processes.iter().map(|p| format!("{}: {}", p.name, p.command.join(" "))).collect::<Vec<_>>().join("; ")
    };
    field("processes", procs(ma), procs(mb));
    let needs = |m: &odi_format::Manifest| m.env.as_ref().map(|e| e.required.join(" ")).unwrap_or_default();
    field("env needs", needs(ma), needs(mb));
    let parts = |m: &odi_format::Manifest| {
        m.parts.iter().map(|p| format!("{} {}", p.name, short(&p.digest))).collect::<Vec<_>>().join(", ")
    };
    field("parts", parts(ma), parts(mb));
    for line in said {
        println!("{line}");
    }

    // The files.
    let files = |rd: &Reader| -> BTreeMap<String, (Kind, u64, Vec<String>, String, u32)> {
        rd.entries
            .iter()
            .filter(|e| !e.path.starts_with("meta/odi/"))
            .map(|e| {
                (
                    e.path.clone(),
                    (e.kind, e.size, e.chunks.iter().map(|c| c.hash.clone()).collect(), e.target.clone(), e.mode),
                )
            })
            .collect()
    };
    let (fa, fb) = (files(&ra), files(&rb));
    let mut changes: Vec<(char, &str, String)> = Vec::new();
    let mut same = 0usize;
    for (p, nb) in &fb {
        match fa.get(p) {
            None => changes.push(('+', p, size_note(nb.0, nb.1))),
            Some(na) if na == nb => same += 1,
            Some(na) => {
                let note = if na.0 == Kind::File && nb.0 == Kind::File {
                    let d = nb.1 as i64 - na.1 as i64;
                    format!(
                        "{} -> {} ({}{})",
                        human(na.1),
                        human(nb.1),
                        if d >= 0 { "+" } else { "-" },
                        human(d.unsigned_abs())
                    )
                } else {
                    "changed".into()
                };
                changes.push(('~', p, note));
            }
        }
    }
    for (p, na) in &fa {
        if !fb.contains_key(p) {
            changes.push(('-', p, size_note(na.0, na.1)));
        }
    }
    changes.sort_by(|x, y| x.1.cmp(y.1));
    let count = |k| changes.iter().filter(|c| c.0 == k).count();
    let total = |rd: &Reader| rd.entries.iter().map(|e| e.size).sum::<u64>();
    let (sa, sb) = (total(&ra), total(&rb));
    let d = sb as i64 - sa as i64;
    println!(
        "  files     +{} -{} ~{} ({same} the same)\n  size      {} -> {} ({}{})",
        count('+'),
        count('-'),
        count('~'),
        human(sa),
        human(sb),
        if d >= 0 { "+" } else { "-" },
        human(d.unsigned_abs())
    );
    // What a machine that has OLD fetches for NEW: its chunks OLD lacks.
    let old: BTreeSet<&str> = ra.entries.iter().flat_map(|e| e.chunks.iter().map(|c| c.hash.as_str())).collect();
    let mut seen = BTreeSet::new();
    let (mut new_bytes, mut all_bytes) = (0u64, 0u64);
    for c in rb.entries.iter().flat_map(|e| e.chunks.iter()) {
        if seen.insert(c.hash.as_str()) {
            all_bytes += c.length;
            if !old.contains(c.hash.as_str()) {
                new_bytes += c.length;
            }
        }
    }
    println!("  to fetch  {} of {} (what's new to a machine that has the old one)", human(new_bytes), human(all_bytes));
    if !changes.is_empty() {
        println!();
        let limit = if f.bool("all") { usize::MAX } else { 40 };
        for (k, p, note) in changes.iter().take(limit) {
            println!("{k} {p}  {note}");
        }
        if changes.len() > limit {
            println!("… {} more (--all lists them)", changes.len() - limit);
        }
    }
    Ok(())
}

fn size_note(k: Kind, size: u64) -> String {
    match k {
        Kind::File => human(size),
        Kind::Dir => "folder".into(),
        Kind::Symlink => "link".into(),
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Listed {
    ns: String,
    name: String,
    #[serde(deserialize_with = "nullable")]
    public: Value,
    updated: i64,
    language: String,
    pulls: u64,
}

fn print_repos(list: &[Listed], visibility: bool) {
    let names: Vec<String> = list.iter().map(|l| format!("{}/{}", l.ns, l.name)).collect();
    let w = names.iter().map(String::len).max().unwrap_or(0);
    let lw = list.iter().map(|l| l.language.len()).max().unwrap_or(0);
    for (l, n) in list.iter().zip(&names) {
        let vis = if !visibility {
            String::new()
        } else if l.public.as_bool() == Some(true) || l.public.as_i64() == Some(1) {
            "public   ".into()
        } else {
            "private  ".into()
        };
        let updated = if l.updated > 0 { format!("updated {}", when(l.updated)) } else { "nothing published".into() };
        println!("{n:<w$}  {vis}{:<lw$}  {:>6} pulls  {updated}", l.language, l.pulls);
    }
}

pub fn cmd_search(args: &[String]) -> Result<()> {
    let q = args.join(" ");
    let c = new_client(&Ref::default());
    let path =
        format!("/v1/search?q={}", percent_encoding::utf8_percent_encode(&q, percent_encoding::NON_ALPHANUMERIC));
    let list: Vec<Listed> = c.json("GET", &path, None)?;
    if list.is_empty() {
        println!("no public repositories{}", if q.is_empty() { " yet".to_string() } else { format!(" match {q:?}") });
        return Ok(());
    }
    print_repos(&list, false);
    Ok(())
}

pub fn cmd_repos(args: &[String]) -> Result<()> {
    if !args.is_empty() {
        bail!("usage: odi repos");
    }
    let c = new_client(&Ref::default());
    if !c.signed_in() {
        bail!("sign in first: odi login");
    }
    let list: Vec<Listed> = c.json("GET", "/v1/mine", None)?;
    if list.is_empty() {
        println!("no repositories yet: odi push NAMESPACE/NAME makes one");
        return Ok(());
    }
    print_repos(&list, true);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(names_tag("acme/web:1.4"));
        assert!(names_tag("hub.example.com:8080/acme/web:1"));
        assert!(!names_tag("hub.example.com:8080/acme/web"));
        assert!(!names_tag("acme/web"));
        assert!(names_tag("acme/web@sha256:abc"));
    }
}
