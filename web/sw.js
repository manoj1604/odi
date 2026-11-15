// The ODI runner's service worker: serves run/KEY/... (under its scope) out of an ODI file (or
// a set), fetching only what's asked for. KEY: base64url of {src, parts}, so
// a worker started afresh can open it again.
//
// Offline: the runner itself (this page, odi.js, odi_web.wasm) is kept when
// the worker installs, and served from the network when it can be, else as
// kept; chunks and what files hold are kept as read (odi.js).
"use strict";
importScripts("odi.js");
const wasmURL = new URL("odi_web.wasm", self.location).href;
const ready = ODI.load(wasmURL);
const apps = new Map();
const shell = ["./", "odi.js", "odi_web.wasm"].map((p) => new URL(p, self.registration.scope).href);

const fromKey = (key) => JSON.parse(atob(key.replace(/-/g, "+").replace(/_/g, "/")));
function app(key) {
  if (!apps.has(key)) {
    const { src, parts } = fromKey(key);
    apps.set(key, ready.then(() => ODI.assemble(src, parts)).catch((e) => { apps.delete(key); throw e; }));
  }
  return apps.get(key);
}

self.addEventListener("install", (e) => {
  e.waitUntil(caches.open(ODI.SHELL).then((c) => c.addAll(shell)).catch(() => {}).then(() => self.skipWaiting()));
});
self.addEventListener("activate", (e) => e.waitUntil(self.clients.claim()));

const isolated = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
  "Cross-Origin-Resource-Policy": "same-origin",
};

// The runner's own files: fresh when there's a network, kept when not.
async function fromShell(req, url) {
  try {
    const res = await fetch(req, { cache: "no-cache" });
    if (res.ok) {
      const copy = res.clone();
      caches.open(ODI.SHELL).then((c) => c.put(url, copy)).catch(() => {});
    }
    return res;
  } catch (err) {
    const kept = await caches.match(url, { cacheName: ODI.SHELL });
    if (kept) return kept;
    throw err;
  }
}

self.addEventListener("fetch", (e) => {
  const u = new URL(e.request.url);
  if (u.origin !== self.location.origin || e.request.method !== "GET") return;
  const scope = self.registration.scope;
  const plain = u.origin + u.pathname;
  if (plain === scope || plain === scope + "index.html") return e.respondWith(fromShell(e.request, scope));
  if (shell.includes(plain)) return e.respondWith(fromShell(e.request, plain));
  const base = new URL("run/", scope).pathname;
  if (!u.pathname.startsWith(base)) return;
  e.respondWith((async () => {
    const [key, ...rest] = u.pathname.slice(base.length).split("/");
    try {
      const a = await app(key);
      const n = ODI.resolve(a, "/" + rest.map(decodeURIComponent).join("/"));
      if (!n) {
        const why = a.manifest.static ? "404 not found\n" : "this is a server program: odi run it\n";
        return new Response(why, { status: 404, headers: { "Content-Type": "text/plain", ...isolated } });
      }
      const body = await n.file.read(n.e);
      return new Response(body, { headers: { "Content-Type": ODI.type(n.e.path), "Cache-Control": "no-store", ...isolated } });
    } catch (err) {
      return new Response(String(err) + "\n", { status: 502, headers: { "Content-Type": "text/plain", ...isolated } });
    }
  })());
});

self.addEventListener("message", (e) => {
  const port = e.ports[0];
  const reply = (m) => port.postMessage(m);
  const fail = (err) => reply({ ok: false, error: String(err && err.message || err) });
  const d = e.data;
  if (d.type === "open") {
    app(d.key).then(
      (a) => reply({ ok: true, name: a.manifest.name, version: a.manifest.version || "", digest: a.digest, offline: a.offline,
        signatures: a.signatures, parts: a.parts, replaced: a.replaced, size: a.size, stats: { ...ODI.stats } }),
      fail);
  } else if (d.type === "stats") {
    reply({ ...ODI.stats });
  } else if (d.type === "status") {
    app(d.key).then(ODI.status).then((s) => reply({ ok: true, ...s }), fail);
  } else if (d.type === "save") {
    // (Progress as it goes, then the end: { done: true }.)
    e.waitUntil(app(d.key)
      .then((a) => ODI.save(a, (n, of) => reply({ ok: true, progress: n, of })).then(() => ODI.status(a)))
      .then((s) => reply({ ok: true, done: true, ...s, stats: { ...ODI.stats } }), fail));
  } else if (d.type === "kept") {
    ready.then(ODI.kept).then((list) => reply({ ok: true, list }), fail);
  }
});
