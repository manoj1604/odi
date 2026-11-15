//! Writes a tiny ODI file (a static site), then opens it again and reads a
//! file back, every byte checked.
//!
//!   cargo run -p odi-format --example hello

use odi_format::{digest_string, Kind, Manifest, Platform, Reader, Static, Writer};

fn main() -> odi_format::Result<()> {
    let path = std::env::temp_dir().join("hello.odi");

    // Write: folders and files, then the manifest; the digest comes back.
    let mut w = Writer::new(std::fs::File::create(&path)?)?;
    w.add_dir("static", 0o755)?;
    w.add_file("static/index.html", 0o644, &b"<h1>hello</h1>\n"[..])?;
    let mut m = Manifest {
        name: "hello".into(),
        version: "1.0".into(),
        platform: Some(Platform { os: "any".into(), arch: "any".into(), ..Default::default() }),
        r#static: vec![Static { from: "static".into(), at: "/".into(), immutable: false }],
        ..Default::default()
    };
    let digest = w.finish(&mut m)?;
    println!("wrote {} {}", path.display(), digest_string(&digest));

    // Read: opening checks the footer, manifest, index and digest; reading a
    // file checks each chunk's hash.
    let rd = Reader::open(&path)?;
    for e in &rd.entries {
        let what = if e.kind == Kind::Dir { "dir " } else { "file" };
        println!("{what} {}", e.path);
    }
    print!("{}", String::from_utf8_lossy(&rd.read_file("static/index.html")?));
    rd.verify()?; // every byte, against the digest
    Ok(())
}
