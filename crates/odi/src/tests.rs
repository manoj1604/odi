use std::fs;
use std::path::Path;

use odi_format::{Entry, Kind, Manifest, Platform, Process, Reader, Writer};

use crate::build::Project;
use crate::cross::{allows, binary_platform, package_of, NodePackage};
use crate::hub::{parse_ref, segments, Ref};
use crate::{extract, host_arch, host_os};

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn project(stage: &Path, os: &str, arch: &str) -> Project {
    Project {
        dir: stage.into(),
        stage: stage.into(),
        goos: os.into(),
        goarch: arch.into(),
        build: false,
        m: Manifest::default(),
    }
}

fn manifest() -> Manifest {
    Manifest {
        name: "t".into(),
        processes: vec![Process { name: "web".into(), command: s(&["node"]), ..Default::default() }],
        ..Default::default()
    }
}

#[test]
fn package_lists() {
    for (list, v, want) in [
        (vec![], "linux", true),
        (s(&["linux"]), "linux", true),
        (s(&["darwin"]), "linux", false),
        (s(&["!win32"]), "linux", true),
        (s(&["!linux"]), "linux", false),
    ] {
        assert_eq!(allows(&list, v), want, "{list:?} {v}");
    }
}

#[test]
fn fits() {
    let p = project(Path::new("/x"), "linux", "amd64");
    let pkg = |os: &[&str], cpu: &[&str], libc: &[&str]| NodePackage {
        os: s(os),
        cpu: s(cpu),
        libc: s(libc),
        ..Default::default()
    };
    assert!(p.fits(&pkg(&["linux"], &["x64"], &["glibc"])), "linux x64 glibc should fit");
    for n in [pkg(&["darwin"], &[], &[]), pkg(&["linux"], &["arm64"], &[]), pkg(&["linux"], &[], &["musl"])] {
        assert!(!p.fits(&n), "{:?} fits", (n.os, n.cpu, n.libc));
    }
}

#[test]
fn package_names() {
    for (path, want) in [
        ("/s/app/node_modules/sharp/build/a.node", "sharp"),
        ("/s/app/node_modules/@img/sharp-linux-x64/lib/a.node", "@img/sharp-linux-x64"),
        ("/s/app/node_modules/a/node_modules/b/x.node", "b"),
        ("/s/app/lib/own.node", "app/lib/own.node"),
    ] {
        assert_eq!(package_of(Path::new("/s"), Path::new(path)), want, "{path}");
    }
}

/// A compiled file is read for what it really is, whatever it's called, and
/// the build refuses one that isn't for the target.
#[test]
fn native_files() {
    let me = fs::read(std::env::current_exe().unwrap()).unwrap();
    let stage = tempfile::tempdir().unwrap();
    let dir = stage.path().join("app/node_modules/bcrypt");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("bcrypt.node"), me).unwrap();
    fs::write(dir.join("not-compiled.node"), "text").unwrap();

    let (os, arches) = binary_platform(&dir.join("bcrypt.node")).unwrap();
    assert!(os == host_os() && arches.iter().any(|a| a == host_arch()), "{os} {arches:?}");
    let here = project(stage.path(), host_os(), host_arch());
    let (n, wrong) = here.native_files().unwrap();
    assert_eq!((n, wrong.len()), (1, 0), "{wrong:?}");
    let other = if host_os() == "linux" { "darwin" } else { "linux" };
    let mut there = project(stage.path(), other, host_arch());
    assert_eq!(there.native_files().unwrap().1.len(), 1);
    assert!(there.platform_from_files().is_err(), "a build with another platform's compiled file went through");
}

/// A chain of links, each fine on its own, that leaves the unpack folder:
/// app/k reads as inside, but on disk a/l is a link, so its ".." climbs.
/// app/k/planted would then be made outside.
const CLIMBING: [(&str, &str); 3] = [("app/a/l", ".."), ("app/k", "a/l/../.."), ("app/k/planted", "../odi-escaped")];

#[test]
fn writer_refuses_climbing_links() {
    let mut out = Vec::new();
    let mut w = Writer::new(&mut out).unwrap();
    for (p, t) in CLIMBING {
        w.add_symlink(p, t).unwrap();
    }
    assert!(w.finish(&mut manifest()).is_err(), "a file whose links leave it was made");
}

/// Unpacking holds by itself, as if the reader had let them through.
#[test]
fn unpack_stays_inside() {
    let entries = CLIMBING
        .iter()
        .map(|(p, t)| {
            let mut e = Entry::new(*p, Kind::Symlink);
            e.target = t.to_string();
            e
        })
        .collect();
    let rd = Reader::from_entries_unchecked(entries);
    let base = tempfile::tempdir().unwrap();
    let r = extract(&rd, &base.path().join("unpacked"));
    assert!(
        fs::symlink_metadata(base.path().join("planted")).is_err(),
        "a link was written outside the folder it was unpacked into"
    );
    assert!(r.is_err(), "unpacked");
}

/// Extracting refuses nothing valid and writes nothing outside its folder.
#[test]
fn extracts() {
    let mut out = Vec::new();
    let mut w = Writer::new(&mut out).unwrap();
    w.add_dir("app", 0o755).unwrap();
    w.add_file("app/run.sh", 0o755, &b"#!/bin/sh\necho hi\n"[..]).unwrap();
    w.add_symlink("app/link", "run.sh").unwrap();
    let mut m = manifest();
    m.processes[0].command = s(&["sh", "run.sh"]);
    w.finish(&mut m).unwrap();
    let n = out.len() as u64;
    let rd = Reader::new(out, n).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().join("x");
    extract(&rd, &dir).unwrap();
    assert_eq!(fs::read(dir.join("app/run.sh")).unwrap(), b"#!/bin/sh\necho hi\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(dir.join("app/run.sh")).unwrap().permissions().mode() & 0o777, 0o755);
    }
    assert_eq!(fs::read_link(dir.join("app/link")).unwrap(), Path::new("run.sh"));
}

#[test]
fn refs() {
    let d = "a".repeat(64);
    let r = |registry: &str, ns: &str, name: &str, tag: &str, digest: &str| Ref {
        registry: registry.into(),
        ns: ns.into(),
        name: name.into(),
        tag: tag.into(),
        digest: digest.into(),
    };
    for (s, want) in [
        ("acme/web".to_string(), r("", "acme", "web", "latest", "")),
        ("acme/web:1.4".into(), r("", "acme", "web", "1.4", "")),
        (format!("acme/web@sha256:{d}"), r("", "acme", "web", "", &d)),
        ("odi.example.com/a/b:main".into(), r("https://odi.example.com", "a", "b", "main", "")),
        ("localhost:9/a/b".into(), r("https://localhost:9", "a", "b", "latest", "")),
    ] {
        assert_eq!(parse_ref(&s).unwrap(), want, "{s}");
    }
    for bad in ["web", "a/b/c", "a/", "/b", "a/b@sha256:short", "a/b:x:y"] {
        assert!(parse_ref(bad).is_err(), "{bad}: accepted");
    }
}

/// The pieces cover the file exactly, and the chunks of what's unchanged are
/// the same pieces in the next version.
#[test]
fn pieces() {
    let build = |extra: &str| -> (Vec<crate::hub::Segment>, u64) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.odi");
        let mut f = fs::File::create(&path).unwrap();
        let mut w = Writer::new(&mut f).unwrap();
        w.add_dir("app", 0o755).unwrap();
        let mut seed = 1u64;
        let big: Vec<u8> = (0..3 << 20)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed as u8
            })
            .collect();
        w.add_file("app/big", 0o644, &big[..]).unwrap();
        w.add_file("app/small", 0o644, extra.as_bytes()).unwrap();
        let mut m = manifest();
        m.platform = Some(Platform { os: "any".into(), arch: "any".into(), libc: String::new() });
        w.finish(&mut m).unwrap();
        drop(f);
        let rd = Reader::open(&path).unwrap();
        (segments(&rd).unwrap(), rd.size())
    };
    let (a, size) = build("one");
    let mut at = 0;
    for sg in &a {
        assert_eq!(sg.off, at, "a gap at {at}");
        at += sg.length;
    }
    assert_eq!(at, size);
    let (b, _) = build("two");
    let have: std::collections::HashSet<&str> = a.iter().map(|s| s.hash.as_str()).collect();
    let same: u64 = b.iter().filter(|s| have.contains(s.hash.as_str())).map(|s| s.length).sum();
    assert!(same >= size / 2, "only {same} of {size} bytes shared between versions");
}

#[test]
fn names_made_safe() {
    use crate::build::safe_name;
    assert_eq!(safe_name("@acme/web app"), "acme-web-app");
    assert_eq!(safe_name("..."), "app");
    assert_eq!(safe_name("a.b_c-d"), "a.b_c-d");
}

/// A start list keeps reads in the order they happened, each chunk once, and
/// lists a file read whole without its chunks.
#[test]
fn start_list_from_reads() {
    let mut out = Vec::new();
    let mut w = Writer::new(&mut out).unwrap();
    let mut seed = 7u64;
    let big: Vec<u8> = (0..6 << 20)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as u8
        })
        .collect();
    w.add_file("app/big", 0o644, &big[..]).unwrap();
    w.add_file("app/small", 0o644, &b"hi"[..]).unwrap();
    w.finish(&mut manifest()).unwrap();
    let n = out.len() as u64;
    let rd = Reader::new(out, n).unwrap();
    let chunks = rd.entry("app/big").unwrap().chunks.len();
    assert!(chunks > 2, "{chunks}");
    let reads: Vec<(String, usize)> =
        vec![("app/small".into(), 0), ("app/big".into(), 1), ("app/small".into(), 0), ("app/big".into(), 0)];
    let s = crate::hints::StartList::from_reads(&reads, &rd);
    let got: Vec<(&str, &[usize])> = s.files.iter().map(|f| (f.path.as_str(), f.chunks.as_slice())).collect();
    assert_eq!(got, [("app/small", &[][..]), ("app/big", &[1, 0][..])]);
    let big = rd.entry("app/big").unwrap();
    let want: std::collections::BTreeSet<(u64, u64)> =
        [&big.chunks[0], &big.chunks[1], &rd.entry("app/small").unwrap().chunks[0]]
            .iter()
            .map(|c| (c.offset, c.length))
            .collect();
    assert_eq!(s.ranges(&rd).into_iter().collect::<std::collections::BTreeSet<_>>(), want);
}

/// Repacking keeps every entry and its content, replaces what it's given, and
/// puts runner hints last in the file, just before the index.
#[test]
fn repack_puts_hints_last() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.odi");
    let mut f = fs::File::create(&a).unwrap();
    let mut w = Writer::new(&mut f).unwrap();
    w.add_dir("app", 0o755).unwrap();
    w.add_file("app/x.js", 0o644, &b"console.log(1)"[..]).unwrap();
    w.add_file("meta/odi/routes.json", 0o644, &br#"{"version":1,"routes":[]}"#[..]).unwrap();
    w.add_file("static/z.css", 0o644, &b"z{}"[..]).unwrap();
    w.add_symlink("app/l", "x.js").unwrap();
    w.finish(&mut manifest()).unwrap();
    drop(f);
    let rd = Reader::open(&a).unwrap();
    let b = dir.path().join("b.odi");
    crate::repack(&rd, &[(crate::hints::START, br#"{"version":1,"files":[]}"#.to_vec())], &b).unwrap();
    let rb = Reader::open(&b).unwrap();
    for p in ["app/x.js", "static/z.css", "meta/odi/routes.json"] {
        assert_eq!(rb.read_file(p).unwrap(), rd.read_file(p).unwrap(), "{p}");
    }
    assert_eq!(rb.entry("app/l").unwrap().target, "x.js");
    assert!(rb.entry(crate::hints::START).is_some());
    let at = |p: &str| rb.entry(p).unwrap().chunks[0].offset;
    let last_other = at("app/x.js").max(at("static/z.css"));
    assert!(at("meta/odi/routes.json") > last_other && at(crate::hints::START) > last_other);
}

/// Writes an ODI file of (path, content) files (a trailing / is a folder).
fn odi_of(path: &Path, m: &mut Manifest, files: &[(&str, &str)]) -> [u8; 32] {
    crate::write_odi(path, m, |w| {
        for (p, c) in files {
            match p.strip_suffix('/') {
                Some(d) => w.add_dir(d, 0o755)?,
                None => w.add_file(p, 0o755, c.as_bytes())?,
            }
        }
        Ok(())
    })
    .unwrap()
}

#[test]
fn sets_put_together() {
    use crate::parts::{assemble, store_add};
    use odi_format::{digest_string, Part, Plug, Runtime};
    let base = tempfile::tempdir().unwrap();
    // (The only test that uses the store.)
    std::env::set_var("ODI_CACHE", base.path().join("cache"));
    let at = |n: &str| base.path().join(n);

    let mut rt = Manifest {
        odi: "0.2".into(),
        name: "node".into(),
        version: "22.1.0".into(),
        provides: vec![Plug {
            plug: "runtime".into(),
            name: "node".into(),
            version: "22.1.0".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let rt_d = odi_of(&at("rt.odi"), &mut rt, &[("runtime/", ""), ("runtime/bin/", ""), ("runtime/bin/node", "N")]);
    let mut app = manifest();
    app.odi = "0.1".into();
    app.runtime = Some(Runtime { name: "node".into(), version: "22".into(), ..Default::default() });
    odi_of(&at("app.odi"), &mut app, &[("app/", ""), ("app/server.js", "old"), ("app/keep.js", "k")]);
    let mut ov = Manifest { odi: "0.2".into(), name: "theme".into(), ..manifest() };
    odi_of(&at("ov.odi"), &mut ov, &[("app/", ""), ("app/server.js", "new")]);
    let mut docs = Manifest { odi: "0.2".into(), name: "docs".into(), ..manifest() };
    odi_of(&at("docs.odi"), &mut docs, &[("static/", ""), ("static/index.html", "hi")]);
    let part = |name: &str, file: &str, role: &str| {
        let d = store_add(&at(file)).unwrap();
        Part { name: name.into(), digest: digest_string(&d), role: role.into(), from: String::new() }
    };

    let mut set = app.clone();
    set.odi = "0.2".into();
    set.runtime.as_mut().unwrap().digest = digest_string(&rt_d);
    set.parts = vec![
        part("app", "app.odi", "main"),
        part("node", "rt.odi", "runtime"),
        part("theme", "ov.odi", "overlay"),
        part("docs", "docs.odi", ""),
    ];
    odi_of(&at("set.odi"), &mut set, &[]);
    let a = assemble(Reader::open(at("set.odi")).unwrap(), None).unwrap();
    let rd = &a.rd;
    assert_eq!(rd.read_file("app/server.js").unwrap(), b"new", "the overlay wins");
    assert_eq!(rd.read_file("app/keep.js").unwrap(), b"k");
    assert_eq!(rd.read_file("runtime/bin/node").unwrap(), b"N");
    assert_eq!(rd.read_file("parts/docs/static/index.html").unwrap(), b"hi");
    assert!(rd.manifest.runtime.as_ref().unwrap().included, "the runtime is in the tree now");
    assert_eq!(a.replaced.len(), 1);
    assert_eq!((a.replaced[0].path.as_str(), a.replaced[0].by.as_str()), ("app/server.js", "theme"));
    rd.verify().unwrap();
    let dir = at("out");
    extract(rd, &dir).unwrap();
    assert_eq!(fs::read(dir.join("parts/docs/static/index.html")).unwrap(), b"hi");

    // A 0.1 file pinning a runtime part finds it in the store.
    let mut pinned = app.clone();
    pinned.runtime.as_mut().unwrap().digest = digest_string(&rt_d);
    odi_of(&at("pinned.odi"), &mut pinned, &[("app/", ""), ("app/server.js", "x")]);
    let a = assemble(Reader::open(at("pinned.odi")).unwrap(), None).unwrap();
    assert_eq!(a.rd.read_file("runtime/bin/node").unwrap(), b"N");
    assert_ne!(a.key, hex::encode(a.rd.digest), "a tree with parts unpacks apart from the file alone");

    // A runtime part that isn't the runtime the app runs on is refused.
    let mut py = rt.clone();
    py.name = "python".into();
    py.provides[0].name = "python".into();
    odi_of(&at("py.odi"), &mut py, &[("runtime/", ""), ("runtime/bin/", ""), ("runtime/bin/python", "P")]);
    let mut bad = set.clone();
    bad.parts[1] = part("node", "py.odi", "runtime");
    odi_of(&at("bad.odi"), &mut bad, &[]);
    let err = assemble(Reader::open(at("bad.odi")).unwrap(), None).err().unwrap();
    assert!(format!("{err:#}").contains("provides the runtime python"), "{err:#}");

    // A part that isn't here and says nowhere to get it.
    let mut lost = set.clone();
    lost.parts[3].digest = format!("sha256:{}", "ab".repeat(32));
    odi_of(&at("lost.odi"), &mut lost, &[]);
    let err = assemble(Reader::open(at("lost.odi")).unwrap(), None).err().unwrap();
    assert!(format!("{err:#}").contains("isn't on this machine"), "{err:#}");
}

#[test]
fn set_manifests_checked() {
    use odi_format::Part;
    let good = || Part { name: "a".into(), digest: format!("sha256:{}", "0".repeat(64)), ..Default::default() };
    let mut m = Manifest { odi: "0.2".into(), name: "s".into(), parts: vec![good()], ..Default::default() };
    m.check().unwrap();
    for bad in [
        Part { name: "a b".into(), ..good() },
        Part { digest: "sha256:xyz".into(), ..good() },
        Part { role: "boss".into(), ..good() },
    ] {
        m.parts = vec![bad.clone()];
        assert!(m.check().is_err(), "{bad:?}");
    }
    m.parts = vec![good(), good()];
    assert!(m.check().is_err(), "two parts with one name");
}

#[test]
fn repos_of_refs() {
    use crate::parts::repo_of;
    assert_eq!(repo_of("acme/node:22"), "acme/node");
    assert_eq!(repo_of("acme/node@sha256:ab"), "acme/node");
    assert_eq!(repo_of("acme/node"), "acme/node");
    assert_eq!(repo_of("localhost:5000/acme/node"), "localhost:5000/acme/node");
}

#[test]
fn piece_cache() {
    use crate::hub::{checked_build, Build, PieceCache, Segment};
    use sha2::{Digest as _, Sha256};
    let dir = tempfile::tempdir().unwrap();
    let c = PieceCache::at(dir.path().to_path_buf());
    let data = b"some piece".to_vec();
    let seg = Segment { hash: hex::encode(Sha256::digest(&data)), length: data.len() as u64, off: 0 };
    assert!(c.get(&seg).is_none());
    c.put(&seg, &data);
    assert_eq!(c.get(&seg).unwrap(), data);
    // A damaged piece is thrown away, not used.
    let p = dir.path().join(&seg.hash[..2]).join(&seg.hash);
    fs::write(&p, b"some pieCe").unwrap();
    assert!(c.get(&seg).is_none());
    assert!(!p.exists());
    // A registry naming a piece as anything but a SHA-256 is refused (the name
    // is a file's name here).
    for bad in ["../../etc/passwd", "AB", &"A".repeat(64)] {
        let b = Build {
            size: 1,
            segments: vec![Segment { hash: bad.to_string(), length: 1, off: 0 }],
            ..Default::default()
        };
        assert!(checked_build(b).is_err(), "{bad}");
    }
    let b = Build { size: 10, segments: vec![seg.clone()], ..Default::default() };
    assert_eq!(checked_build(b).unwrap().segments[0].off, 0);
}

#[test]
fn plug_vars() {
    use crate::run::plug_var;
    assert_eq!(plug_var("prices"), "ODI_PLUG_PRICES_URL");
    assert_eq!(plug_var("pdf-reader.v2"), "ODI_PLUG_PDF_READER_V2_URL");
}
