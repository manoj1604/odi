//! odi web: the browser runner (web/) and ODI files to run in it, served
//! here: http://localhost:PORT/?src=files/NAME.odi. Files are served with
//! Range requests, so the tab fetches only what the app asks for; a set's
//! parts come from this machine's store (parts/HEX.odi). Any static host
//! with Range requests serves the same way.

use std::collections::HashMap;
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::header::{self, HeaderValue};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use odi_format::{digest_string, Reader};
use tower::ServiceExt;

use crate::flags::{self, flag, Kind::*};

const INDEX: &[u8] = include_bytes!("../../../web/index.html");
const SW: &[u8] = include_bytes!("../../../web/sw.js");
const LIB: &[u8] = include_bytes!("../../../web/odi.js");
const WASM: &[u8] = include_bytes!("../../../web/odi_web.wasm");

type BoxError = Box<dyn std::error::Error + Send + Sync>;
type Body = http_body_util::combinators::UnsyncBoxBody<Bytes, BoxError>;

pub fn cmd_web(args: &[String]) -> Result<()> {
    const U: &str = "odi web FILE... [--port 8080]";
    let specs = [flag("port", Value, "the port (default 8080)")];
    let (f, pos) = flags::parse(&specs, args, U)?;
    if pos.is_empty() {
        bail!("usage: {U}");
    }
    let port = u16::try_from(f.int("port")?.unwrap_or(8080)).context("--port")?;
    let mut files = HashMap::new();
    for p in &pos {
        let path = std::fs::canonicalize(p).with_context(|| p.clone())?;
        let rd = Reader::open(&path).with_context(|| p.clone())?;
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("app.odi").to_string();
        // (Its parts, from wherever odi can get them, into the store: the tab
        // asks for them by digest.)
        if !rd.manifest.parts.is_empty() {
            crate::parts::assemble(Reader::open(&path)?, None)?;
            for part in &rd.manifest.parts {
                let d = part.digest_bytes().expect("checked");
                if !crate::parts::store_path(&d)?.exists() {
                    eprintln!(
                        "odi: part {} isn't in this machine's store (from a registry: odi pull it, then odi part add)",
                        part.name
                    );
                }
            }
        }
        println!("http://localhost:{port}/?src=files/{name}  {} {}", rd.manifest.name, digest_string(&rd.digest));
        files.insert(name, path);
    }
    let files = Arc::new(files);
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(async move {
        let listener =
            tokio::net::TcpListener::bind(("127.0.0.1", port)).await.with_context(|| format!("port {port}"))?;
        loop {
            let (stream, _) = listener.accept().await?;
            let files = files.clone();
            tokio::spawn(async move {
                let svc = hyper::service::service_fn(move |req| handle(files.clone(), req));
                let _ = hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(stream), svc).await;
            });
        }
    })
}

fn bytes(status: StatusCode, ctype: &'static str, b: &'static [u8]) -> Response<Body> {
    let mut r = Response::new(Full::new(Bytes::from_static(b)).map_err(|e| match e {}).boxed_unsync());
    *r.status_mut() = status;
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(ctype));
    r
}

/// The runner page and what it runs are cross-origin isolated, so apps that
/// need SharedArrayBuffer (threads: Godot, Unity, Emscripten) work.
fn isolated(mut r: Response<Body>) -> Response<Body> {
    let h = r.headers_mut();
    h.insert("Cross-Origin-Opener-Policy", HeaderValue::from_static("same-origin"));
    h.insert("Cross-Origin-Embedder-Policy", HeaderValue::from_static("require-corp"));
    h.insert("Cross-Origin-Resource-Policy", HeaderValue::from_static("same-origin"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    r
}

async fn handle(
    files: Arc<HashMap<String, PathBuf>>,
    req: Request<hyper::body::Incoming>,
) -> Result<Response<Body>, Infallible> {
    let path = req.uri().path().to_string();
    let r = match path.as_str() {
        "/" | "/index.html" => bytes(StatusCode::OK, "text/html; charset=utf-8", INDEX),
        "/sw.js" => bytes(StatusCode::OK, "text/javascript", SW),
        "/odi.js" => bytes(StatusCode::OK, "text/javascript", LIB),
        "/odi_web.wasm" => bytes(StatusCode::OK, "application/wasm", WASM),
        p => {
            let file = if let Some(name) = p.strip_prefix("/files/") {
                files.get(name).cloned()
            } else if let Some(hex) = p.strip_prefix("/parts/").and_then(|n| n.strip_suffix(".odi")) {
                let mut d = [0u8; 32];
                match hex::decode_to_slice(hex, &mut d) {
                    Ok(()) => crate::parts::store_path(&d).ok().filter(|p| p.exists()),
                    Err(_) => None,
                }
            } else {
                None
            };
            match file {
                Some(f) => serve_file(&f, req).await,
                None => bytes(StatusCode::NOT_FOUND, "text/plain", b"404 not found\n"),
            }
        }
    };
    Ok(isolated(r))
}

/// A file, with Range requests.
async fn serve_file(p: &Path, req: Request<hyper::body::Incoming>) -> Response<Body> {
    let mut r = Request::new(http_body_util::Empty::<Bytes>::new());
    *r.method_mut() = req.method().clone();
    *r.uri_mut() = req.uri().clone();
    *r.headers_mut() = req.headers().clone();
    let Ok(res) = tower_http::services::ServeFile::new(p).oneshot(r).await;
    let mut res = res.map(|b| b.map_err(|e| Box::new(e) as BoxError).boxed_unsync());
    res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/vnd.odi.v0"));
    res
}
