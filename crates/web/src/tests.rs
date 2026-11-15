use super::*;

const FIXTURE: &[u8] = include_bytes!("../tests/fixture.odi");

fn out() -> Vec<u8> {
    OUT.with(|o| o.borrow().clone())
}

fn open_with(tail_len: usize) -> (i32, Vec<u8>) {
    let f = FIXTURE;
    let tail = &f[f.len() - tail_len..];
    let code = unsafe { odi_open(f.len() as f64, f.as_ptr(), 64, tail.as_ptr(), tail.len()) };
    (code, out())
}

#[test]
fn opens_and_reads() {
    let (code, json) = open_with(FIXTURE.len() - 64);
    assert_eq!(code, 0, "{}", String::from_utf8_lossy(&json));
    let v: serde_json::Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(v["digest"], "sha256:66c103ef3118b6101daab937a36900dade1978b0a80042e4effa56f0c5549ba9");
    assert_eq!(v["manifest"]["name"], "fixture");
    let entries = v["entries"].as_array().unwrap();
    let file = |p: &str| -> Vec<u8> {
        let e = entries.iter().find(|e| e["path"] == p).unwrap();
        let mut all = Vec::new();
        for c in e["chunks"].as_array().unwrap() {
            let (off, len) = (c["offset"].as_u64().unwrap() as usize, c["length"].as_u64().unwrap() as usize);
            let cj = serde_json::to_vec(c).unwrap();
            let code = unsafe { odi_chunk(FIXTURE[off..].as_ptr(), len, cj.as_ptr(), cj.len()) };
            assert_eq!(code, 0, "{}", String::from_utf8_lossy(&out()));
            all.extend(out());
        }
        all
    };
    assert_eq!(file("static/index.html"), b"hello from a fixture\n");
    let big = file("static/big.bin");
    assert_eq!(big.len(), 300000);
    assert!(big.iter().enumerate().all(|(i, &b)| b as usize == (i * 7919) % 251));
}

#[test]
fn asks_for_more_tail() {
    let (code, from) = open_with(200);
    assert_eq!(code, 2, "{}", String::from_utf8_lossy(&from));
    let from: usize = String::from_utf8(from).unwrap().parse().unwrap();
    assert!(from < FIXTURE.len() - 200);
    let (code, _) = open_with(FIXTURE.len() - from);
    assert_eq!(code, 0);
}

#[test]
fn refuses_damage() {
    let mut f = FIXTURE.to_vec();
    let n = f.len();
    // (A byte of the digest in the footer: it no longer matches the contents.)
    f[n - 128 + 48] ^= 1;
    let code = unsafe { odi_open(n as f64, f.as_ptr(), 64, f[64..].as_ptr(), n - 64) };
    assert_eq!(code, 1);
    let bad = br#"{"hash":"00","offset":0,"length":10,"size":10}"#;
    let code = unsafe { odi_chunk(b"0123456789".as_ptr(), 10, bad.as_ptr(), bad.len()) };
    assert_eq!(code, 1);
}
