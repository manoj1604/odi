use super::*;

struct F {
    path: &'static str,
    data: Vec<u8>,
    mode: u32,
}

fn f(path: &'static str, data: impl Into<Vec<u8>>, mode: u32) -> F {
    F { path, data: data.into(), mode }
}

fn build(m: &Manifest, files: &[F], dirs: &[&str]) -> (Vec<u8>, [u8; 32]) {
    let mut out = Vec::new();
    let mut w = Writer::new(&mut out).unwrap();
    for d in dirs {
        w.add_dir(d, 0o755).unwrap();
    }
    for f in files {
        w.add_file(f.path, f.mode, &f.data[..]).unwrap();
    }
    let d = w.finish(&mut m.clone()).unwrap();
    (out, d)
}

fn manifest() -> Manifest {
    Manifest {
        name: "t".into(),
        processes: vec![Process {
            name: "web".into(),
            command: vec!["node".into(), "server.js".into()],
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// A fixed pseudo-random stream (xorshift), for data that doesn't compress.
fn noise(n: usize, mut seed: u64) -> Vec<u8> {
    let mut v = Vec::with_capacity(n);
    while v.len() < n {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        v.extend_from_slice(&seed.to_le_bytes());
    }
    v.truncate(n);
    v
}

#[test]
fn round_trip_and_digest() {
    let big = noise(9 << 20, 1);
    let mut files = vec![
        f("app/server.js", "console.log('hi')\n", 0o644),
        f("app/big.bin", big.clone(), 0o644),
        f("app/same.bin", big.clone(), 0o600), // (its chunks are stored once)
        f("app/empty", "", 0o644),
        f("static/index.html", "<h1>hi</h1>", 0o644),
    ];
    let (b, d) = build(&manifest(), &files, &["app", "static"]);
    assert!(b.len() <= big.len() + (1 << 20), "{} bytes: the repeated file was stored twice", b.len());
    let rd = Reader::new(b.clone(), b.len() as u64).unwrap();
    assert_eq!(rd.digest, d);
    assert_eq!(rd.manifest.name, "t");
    for f in &files {
        assert_eq!(rd.read_file(f.path).unwrap(), f.data, "{}", f.path);
    }
    rd.verify().unwrap();

    // Same files, other order: the same digest (§6), other bytes.
    let rev: Vec<F> = files.iter().rev().map(|x| F { path: x.path, data: x.data.clone(), mode: x.mode }).collect();
    let (b2, d2) = build(&manifest(), &rev, &["static", "app"]);
    assert_eq!(d2, d, "the same contents gave another digest");
    assert_ne!(b, b2, "expected another layout");
    // Other content: another digest.
    files[0].data = b"console.log('bye')\n".to_vec();
    let (_, d3) = build(&manifest(), &files, &["app", "static"]);
    assert_ne!(d3, d, "other contents, same digest");
}

#[test]
fn tampering_is_caught() {
    let data = "abcdefgh".repeat(1 << 16);
    let (b, _) = build(&manifest(), &[f("app/a.txt", data, 0o644)], &[]);
    let rd = Reader::new(b.clone(), b.len() as u64).unwrap();
    let off = rd.entry("app/a.txt").unwrap().chunks[0].offset as usize;
    let mut bad = b.clone();
    bad[off + 10] ^= 0xff;
    let rd2 = Reader::new(bad.clone(), bad.len() as u64).unwrap(); // (the index and manifest are intact)
    assert!(rd2.read_file("app/a.txt").is_err(), "a damaged chunk was read");
    // The footer's digest changed: refused at once.
    let mut bad = b.clone();
    let n = bad.len();
    bad[n - FOOTER_SIZE as usize + 48] ^= 1;
    let err = Reader::new(bad, n as u64).err().expect("a wrong digest was accepted");
    assert!(err.0.contains("digest"), "{err}");
    let cut = b[..b.len() - 1].to_vec();
    assert!(Reader::new(cut, b.len() as u64 - 1).is_err(), "a cut file was accepted");
}

#[test]
fn entries_are_checked() {
    let e = |path: &str, kind: Kind, mode: u32, size: u64, target: &str| Entry {
        path: path.into(),
        kind,
        mode,
        size,
        chunks: vec![],
        target: target.into(),
    };
    let bad = [
        e("/etc/passwd", Kind::File, 0, 0, ""),
        e("app/../etc", Kind::Dir, 0, 0, ""),
        e("etc/x", Kind::Dir, 0, 0, ""),
        e("app/x", Kind::Dir, 0o4755, 0, ""),
        e("app/l", Kind::Symlink, 0, 0, "/etc/passwd"),
        e("app/l", Kind::Symlink, 0, 0, "../../etc/passwd"),
        e("app/f", Kind::File, 0, 5, ""),
    ];
    for x in &bad {
        assert!(check_entry(x).is_err(), "{x:?} was accepted");
    }
    let good = [
        e("app/node_modules/.bin/next", Kind::Symlink, 0, 0, "../next/dist/bin/next"),
        e("static/a", Kind::Symlink, 0, 0, "../app/x"),
        e("app", Kind::Dir, 0o755, 0, ""),
    ];
    for x in &good {
        check_entry(x).unwrap_or_else(|err| panic!("{x:?}: {err}"));
    }
}

#[test]
fn static_only_and_manifest_checks() {
    let st = vec![Static { from: "static".into(), at: "/".into(), immutable: false }];
    let m = Manifest { name: "site".into(), r#static: st.clone(), ..Default::default() };
    let (b, _) = build(&m, &[f("static/index.html", "hi", 0o644)], &[]);
    Reader::new(b.clone(), b.len() as u64).unwrap();
    let proc_ = |name: &str, cmd: &[&str]| Process {
        name: name.into(),
        command: cmd.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    for mut bad in [
        Manifest { name: "x".into(), ..Default::default() },
        Manifest { name: "bad name".into(), r#static: st.clone(), ..Default::default() },
        Manifest { name: "x".into(), processes: vec![proc_("web", &[])], ..Default::default() },
        Manifest { name: "x".into(), processes: vec![proc_("a", &["a"]), proc_("a", &["b"])], ..Default::default() },
        Manifest {
            name: "x".into(),
            r#static: vec![Static { from: "../x".into(), at: "/".into(), immutable: false }],
            ..Default::default()
        },
    ] {
        bad.odi = "0.1".into();
        assert!(bad.check().is_err(), "{bad:?} was accepted");
    }
}

#[test]
fn canonical_json() {
    let v = br#" {"b":1, "a":["<&>",2.5,true,null],"c":{"z":"\u00e9","y":"\n"},"n":[1.50,1e3,-0,1E-2],"b":2} "#;
    assert_eq!(
        String::from_utf8(canonical(v).unwrap()).unwrap(),
        r#"{"a":["<&>",2.5,true,null],"b":2,"c":{"y":"\n","z":"é"},"n":[1.50,1e3,-0,1E-2]}"#
    );
}

/// Strings are escaped exactly as Go's encoding/json (HTML escaping off)
/// escapes them, which is what published files were digested with. (The
/// expected output was printed by Go 1.27.)
#[test]
fn strings_as_go_writes_them() {
    let mut s: String = (0u8..0x80).map(char::from).collect();
    s.push_str("é\u{2028}\u{2029}😀");
    let mut b = Vec::new();
    canonical::write_string(&mut b, &s);
    let go = "\"\\u0000\\u0001\\u0002\\u0003\\u0004\\u0005\\u0006\\u0007\\b\\t\\n\\u000b\\f\\r\\u000e\\u000f\\u0010\\u0011\\u0012\\u0013\\u0014\\u0015\\u0016\\u0017\\u0018\\u0019\\u001a\\u001b\\u001c\\u001d\\u001e\\u001f !\\\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\\\]^_`abcdefghijklmnopqrstuvwxyz{|}~\u{7f}é\\u2028\\u2029😀\"";
    assert_eq!(String::from_utf8(b).unwrap(), go);
}

#[test]
fn links_that_leave() {
    let check = |name: &str, want_bad: bool, links: &[(&str, &str)]| {
        let mut out = Vec::new();
        let mut w = Writer::new(&mut out).unwrap();
        for (p, t) in links {
            w.add_symlink(p, t).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        let r = w.finish(&mut manifest());
        assert_eq!(r.is_err(), want_bad, "{name}: {r:?}");
    };
    check("plain", false, &[("app/a", "b"), ("app/x/y", "../a"), ("app/up", "../meta/z")]);
    check("through a link", false, &[("app/d", "../meta"), ("app/e", "d/f")]);
    check("broken", false, &[("app/a", "missing")]);
    check("climbs through a link", true, &[("app/a/l", ".."), ("app/k", "a/l/../..")]);
    check("a loop", true, &[("app/a", "b"), ("app/b", "a")]);
    check("under a link", true, &[("app/k", "z"), ("app/k/x", "../z")]);
}

/// A file around any manifest and index, its digest right, so a test gets
/// past the digest to what's behind it.
fn assemble(mj: &[u8], ij: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut h = [0u8; HEADER_SIZE as usize];
    h[..8].copy_from_slice(MAGIC);
    out.extend_from_slice(&h);
    out.extend_from_slice(b"0123456789abcdef0123456789abcdef");
    let mut foot = [0u8; FOOTER_SIZE as usize];
    let mz = zstd::bulk::compress(mj, 3).unwrap();
    let iz = zstd::bulk::compress(ij, 3).unwrap();
    foot[0..8].copy_from_slice(&(out.len() as u64).to_le_bytes());
    foot[8..16].copy_from_slice(&(mz.len() as u64).to_le_bytes());
    out.extend_from_slice(&mz);
    foot[16..24].copy_from_slice(&(out.len() as u64).to_le_bytes());
    foot[24..32].copy_from_slice(&(iz.len() as u64).to_le_bytes());
    out.extend_from_slice(&iz);
    #[derive(serde::Deserialize)]
    struct Idx {
        #[serde(default)]
        entries: Vec<Entry>,
    }
    if let Ok(idx) = serde_json::from_slice::<Idx>(ij) {
        if let Ok(d) = digest(mj, &idx.entries) {
            foot[48..80].copy_from_slice(&d);
        }
    }
    foot[120..].copy_from_slice(MAGIC);
    out.extend_from_slice(&foot);
    out
}

#[test]
fn chunks_bounded() {
    // (Hand-made: a chunk claiming more stored bytes than any content needs.)
    let e = Entry {
        path: "app/a".into(),
        kind: Kind::File,
        mode: 0o644,
        size: 4,
        chunks: vec![Chunk { hash: "0".repeat(64), offset: HEADER_SIZE, length: 1 << 30, size: 4 }],
        target: String::new(),
    };
    let ij = serde_json::to_vec(&serde_json::json!({ "entries": [e] })).unwrap();
    let mut m = manifest();
    m.odi = "0.1".into();
    let mj = serde_json::to_vec(&m).unwrap();
    let b = assemble(&mj, &ij);
    let mut padded = b[..HEADER_SIZE as usize].to_vec();
    padded.extend(std::iter::repeat_n(0, 1 << 20));
    padded.extend_from_slice(&b[HEADER_SIZE as usize..]);
    // (Offsets moved: fix the footer's pointers.)
    let n = padded.len();
    for i in [0, 16] {
        let at = n - FOOTER_SIZE as usize + i;
        let v = u64::from_le_bytes(padded[at..at + 8].try_into().unwrap()) + (1 << 20);
        padded[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }
    let err = Reader::new(padded, n as u64).err().expect("accepted");
    assert!(err.0.contains("outside"), "{err}");
}

fn exercise(b: Vec<u8>) {
    let n = b.len() as u64;
    let Ok(rd) = Reader::new(b, n) else { return };
    let _ = rd.verify();
    for e in &rd.entries {
        if e.kind == Kind::File {
            let _ = rd.read_file(&e.path);
        }
    }
}

/// Any bytes at all: an error, never a panic. (Go fuzzed this; here, a fixed
/// set of mutations of a real file and of hand-made indexes.)
#[test]
fn hostile_bytes_never_panic() {
    let mut w = Writer::new(Vec::new()).unwrap();
    w.add_file("app/a", 0o644, &b"hello hello hello hello"[..]).unwrap();
    w.add_symlink("app/l", "a").unwrap();
    let mut out = Vec::new();
    {
        let mut w2 = Writer::new(&mut out).unwrap();
        w2.add_file("app/a", 0o644, &b"hello hello hello hello"[..]).unwrap();
        w2.add_symlink("app/l", "a").unwrap();
        w2.finish(&mut manifest()).unwrap();
    }
    drop(w);
    let mut seed = 0x9e3779b97f4a7c15u64;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..3000 {
        let mut b = out.clone();
        for _ in 0..1 + rnd() % 4 {
            let i = (rnd() as usize) % b.len();
            b[i] = rnd() as u8;
        }
        if rnd() % 10 == 0 {
            let cut = (rnd() as usize) % b.len();
            b.truncate(cut);
        }
        exercise(b);
    }
    let mj = br#"{"odi":"0.1","name":"t","processes":[{"name":"web","command":["node"]}],"writable":["app/w/x"]}"#;
    let ij1 = br#"{"entries":[{"path":"app/a","type":"file","mode":420,"size":4,"chunks":[{"hash":"0000000000000000000000000000000000000000000000000000000000000000","offset":64,"length":8,"size":4}]},{"path":"app/l","type":"symlink","target":"a"}]}"#;
    let ij2 = br#"{"entries":[{"path":"app/a/l","type":"symlink","target":".."},{"path":"app/k","type":"symlink","target":"a/l/../.."}]}"#;
    let ij3 = br#"{"entries":[{"path":"app/w","type":"file","mode":420},{"path":"app/x","type":"file","size":4,"chunks":[{"hash":"0000000000000000000000000000000000000000000000000000000000000000","offset":64,"length":4,"size":4}]}]}"#;
    for ij in [&ij1[..], &ij2[..], &ij3[..]] {
        exercise(assemble(mj, ij));
        for _ in 0..500 {
            let mut j = ij.to_vec();
            let i = (rnd() as usize) % j.len();
            j[i] = b"{}[]\":,0123456789abc./-"[(rnd() % 23) as usize];
            exercise(assemble(mj, &j));
        }
    }
}

#[test]
fn canonical_reads_strings_as_go() {
    let c = |j: &[u8]| String::from_utf8(canonical(j).unwrap()).unwrap();
    assert_eq!(c(br#""\ud83d\ude00""#), "\"😀\"");
    assert_eq!(c(br#""\ud83d!""#), "\"\u{fffd}!\"");
    assert_eq!(c(b"\"a\xffb\""), "\"a\u{fffd}b\"");
    assert_eq!(c(br#""\/""#), "\"/\"");
    for bad in [&b"01"[..], b"1.", b"-", b"\"\x01\"", b"[1,]", b"{\"a\"}", b"tru", b"1 2", b"\"\\x\""] {
        assert!(canonical(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
    }
}

#[test]
fn clean_as_go() {
    for (p, want) in
        [("a/b/../c", "a/c"), ("a/../..", ".."), ("./a//b/", "a/b"), ("a/..", "."), ("../a/../../b", "../../b")]
    {
        assert_eq!(clean(p), want, "{p}");
    }
}

#[test]
fn signatures() {
    let (file, d) = build(&manifest(), &[f("app/server.js", "hi", 0o644)], &["app"]);
    let rd = Reader::new(file.clone(), file.len() as u64).unwrap();
    assert!(rd.signatures.is_empty());
    let (a, b) = ([7u8; 32], [9u8; 32]);

    // Signed: the same digest, the same contents, a signature that checks.
    let mut one = Vec::new();
    write_signed(&rd, &Signatures { signatures: vec![sign(&a, &d)] }, &mut one).unwrap();
    let rd1 = Reader::new(one.clone(), one.len() as u64).unwrap();
    assert_eq!(rd1.digest, d);
    assert_eq!(rd1.read_file("app/server.js").unwrap(), b"hi");
    assert_eq!(rd1.signatures.len(), 1);
    rd1.signatures[0].check(&d).unwrap();
    assert_eq!(rd1.signatures[0].key, key_string(&public_key(&a)));

    // Signed again: the old block replaced, both signatures kept.
    let mut two = Vec::new();
    let sigs = Signatures { signatures: vec![rd1.signatures[0].clone(), sign(&b, &d)] };
    write_signed(&rd1, &sigs, &mut two).unwrap();
    let rd2 = Reader::new(two.clone(), two.len() as u64).unwrap();
    assert_eq!(rd2.signatures.len(), 2);
    assert!(rd2.signatures.iter().all(|s| s.check(&d).is_ok()));
    assert!(two.len() < one.len() + 400, "the old block isn't kept: {} then {}", one.len(), two.len());

    // A signature of another digest, or damaged, or under another key, fails.
    let other = [1u8; 32];
    assert!(sign(&a, &other).check(&d).is_err());
    let mut bad = sign(&a, &d);
    bad.sig.replace_range(0..2, if &bad.sig[0..2] == "00" { "11" } else { "00" });
    assert!(bad.check(&d).is_err());
    let mut swapped = sign(&a, &d);
    swapped.key = key_string(&public_key(&b));
    assert!(swapped.check(&d).is_err());
    assert!(parse_key("ed25519:zz").is_err());
}
