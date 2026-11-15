# odi-format

Read and write [ODI files](https://github.com/manoj1604/odi): one file
holding a program's build output, for any language, with a manifest that
says how to run it.

This crate is the format itself (the
[specification](https://github.com/manoj1604/odi/blob/main/SPEC.md)): it
writes files, and reads them with every check a reader must make (footer,
manifest, index, digest, each chunk's hash, signatures). It does no I/O
beyond what it's given to read from, so registries, runners and other tools
can use it.

```rust
use odi_format::{digest_string, Manifest, Platform, Reader, Static, Writer};

let mut w = Writer::new(std::fs::File::create("hello.odi")?)?;
w.add_dir("static", 0o755)?;
w.add_file("static/index.html", 0o644, &b"<h1>hello</h1>\n"[..])?;
let mut m = Manifest {
    name: "hello".into(),
    platform: Some(Platform { os: "any".into(), arch: "any".into(), ..Default::default() }),
    r#static: vec![Static { from: "static".into(), at: "/".into(), immutable: false }],
    ..Default::default()
};
let digest = w.finish(&mut m)?;
println!("{}", digest_string(&digest));

let rd = Reader::open("hello.odi")?;          // checks the digest
let page = rd.read_file("static/index.html")?; // checks each chunk
rd.verify()?;                                  // every byte
```

The whole example is in `examples/hello.rs`
(`cargo run -p odi-format --example hello`).

Features: `zstd` (default) reads and writes with the zstd C library;
`pure` reads only, in pure Rust (for WebAssembly).

Licence: Apache-2.0.
