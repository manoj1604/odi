// ODI files in a browser: opened from their head and tail, read chunk by
// chunk with HTTP Range requests, every byte checked (odi_web.wasm: the
// same Rust code as odi's reader). Sets are put together as odi run does.
// Loaded by the service worker (sw.js) with importScripts.
//
// Offline: every chunk read is kept (Cache Storage, by its hash, as stored:
// compressed) and checked again whenever it's used; what a file holds (its
// manifest and index) is kept by URL, so a file opens with no network.
// save() fetches the rest of a file ahead (odi fill --all, in a tab). Files
// picked from this machine are kept whole (local/), read the same way.
"use strict";
const ODI = (() => {
  let wasm = null;
  const stats = { bytes: 0, requests: 0, size: 0, stored: 0 };
  const base = new URL("odi-store/", self.location).href;
  const CHUNKS = "odi-chunks-v1", FILES = "odi-files-v1", LOCAL = "odi-local-v1", SHELL = "odi-runner-v1";
  const chunkURL = (hash) => base + "chunk/" + hash;
  const isLocal = (url) => url.startsWith(base + "local/");

  // The hashes kept, read once.
  let known = null;
  function keptHashes() {
    if (!known) {
      known = caches.open(CHUNKS).then(async (c) => new Set((await c.keys()).map((r) => r.url.slice(base.length + 6))))
        .catch(() => new Set());
    }
    return known;
  }

  // The reader itself: from the network when it can be (and kept), else as kept.
  async function load(url) {
    if (wasm) return wasm;
    let b;
    try {
      const res = await fetch(url, { cache: "no-cache" });
      if (!res.ok) throw new Error(`${url}: ${res.status}`);
      b = await res.arrayBuffer();
      caches.open(SHELL).then((c) => c.put(url, new Response(b, { headers: { "Content-Type": "application/wasm" } }))).catch(() => {});
    } catch (err) {
      const kept = await caches.match(url);
      if (!kept) throw err;
      b = await kept.arrayBuffer();
    }
    const { instance } = await WebAssembly.instantiate(b, {});
    wasm = instance.exports;
    return wasm;
  }

  function put(bytes) {
    const p = wasm.odi_alloc(bytes.length);
    new Uint8Array(wasm.memory.buffer, p, bytes.length).set(bytes);
    return [p, bytes.length];
  }
  const out = () => new Uint8Array(wasm.memory.buffer, wasm.odi_out_ptr(), wasm.odi_out_len()).slice();
  const text = (b) => new TextDecoder().decode(b);

  // Bytes [start, end) of url, or (start < 0) its last -start bytes; and its size.
  class Offline extends Error {}
  const blobs = new Map();
  async function range(url, start, end) {
    if (isLocal(url)) {
      if (!blobs.has(url)) {
        blobs.set(url, caches.open(LOCAL).then((c) => c.match(url)).then((r) => {
          if (!r) throw new Error("this file isn't kept here any more: pick it again");
          return r.blob();
        }));
      }
      const blob = await blobs.get(url).catch((e) => { blobs.delete(url); throw e; });
      const s = start < 0 ? Math.max(0, blob.size + start) : start;
      const bytes = new Uint8Array(await blob.slice(s, start < 0 ? blob.size : end).arrayBuffer());
      return { bytes, size: blob.size };
    }
    const get = (h) => fetch(url, h).catch((e) => { throw new Offline(`${url}: ${e.message}`); });
    let r = await get({ headers: { Range: start < 0 ? `bytes=${start}` : `bytes=${start}-${end - 1}` } });
    // (A range past a small file's end: some servers refuse it. All of it, then.)
    if (r.status === 416) r = await get();
    if (!r.ok) throw new Error(`${url}: ${r.status}`);
    const b = new Uint8Array(await r.arrayBuffer());
    stats.bytes += b.length;
    stats.requests++;
    const cr = r.headers.get("Content-Range");
    const size = cr ? Number(cr.split("/")[1]) : b.length;
    if (r.status === 200) {
      // (No ranges here: the whole file came. Fine for small files.)
      return { bytes: start < 0 ? b.slice(Math.max(0, b.length + start)) : b.slice(start, end), size };
    }
    return { bytes: b, size };
  }

  class File {
    // Opened from the network (and what it holds kept); with no network, as kept.
    static async open(url) {
      try {
        const f = await File.fetchOpen(url);
        caches.open(FILES).then((c) => c.put(url, new Response(JSON.stringify(
          { size: f.size, info: { digest: f.digest, manifest: f.manifest, entries: f.entries, signatures: f.signatures } }),
          { headers: { "Content-Type": "application/json" } }))).catch(() => {});
        return f;
      } catch (err) {
        if (!(err instanceof Offline)) throw err; // (no network; anything else is real)
        const kept = await caches.match(url, { cacheName: FILES });
        if (!kept) throw new Error(`${url}: no network, and not kept for offline`);
        const { size, info } = await kept.json();
        stats.size += size;
        const f = new File(url, size, info);
        f.offline = true;
        return f;
      }
    }
    static async fetchOpen(url) {
      const [h, t] = await Promise.all([range(url, 0, 64), range(url, -65536)]);
      const size = t.size;
      stats.size += size;
      let tail = t.bytes;
      for (;;) {
        const [hp, hl] = put(h.bytes), [tp, tl] = put(tail);
        const code = wasm.odi_open(size, hp, hl, tp, tl);
        const o = out();
        wasm.odi_free(hp, hl);
        wasm.odi_free(tp, tl);
        if (code === 0) return new File(url, size, JSON.parse(text(o)));
        if (code === 1) throw new Error(`${url}: ${text(o)}`);
        const from = Number(text(o)); // (the tail doesn't reach the index: more of it)
        const more = await range(url, from, size - tail.length);
        const all = new Uint8Array(more.bytes.length + tail.length);
        all.set(more.bytes);
        all.set(tail, more.bytes.length);
        tail = all;
      }
    }
    constructor(url, size, info) {
      Object.assign(this, { url, size, digest: info.digest, manifest: info.manifest, signatures: info.signatures });
      this.entries = info.entries;
      this.chunks = new Map(); // hash -> Promise<bytes>
    }
    // A chunk as stored, checked: its content.
    check(c, stored, what) {
      const [p, n] = put(stored);
      const [cp, cn] = put(new TextEncoder().encode(JSON.stringify({ ...c, offset: 0, length: stored.length })));
      const code = wasm.odi_chunk(p, n, cp, cn);
      const o = out();
      wasm.odi_free(p, n);
      wasm.odi_free(cp, cn);
      if (code !== 0) throw new Error(`${what}: ${text(o)}`);
      return o;
    }
    // Chunks fetched in runs of neighbours (at most gap bytes apart, up to
    // max bytes a run), each checked and kept; this.chunks has them.
    fetchRuns(chunks, what, gap = 0, max = 1 << 30) {
      const want = [...new Map(chunks.filter((c) => !this.chunks.has(c.hash)).map((c) => [c.hash, c])).values()]
        .sort((a, b) => a.offset - b.offset);
      const runs = [];
      for (const c of want) {
        const r = runs[runs.length - 1];
        if (r && c.offset >= r.end && c.offset - r.end <= gap && c.offset + c.length - r.start <= max) {
          r.list.push(c);
          r.end = c.offset + c.length;
        } else {
          runs.push({ start: c.offset, end: c.offset + c.length, list: [c] });
        }
      }
      const local = isLocal(this.url);
      return runs.map((r) => {
        const got = range(this.url, r.start, r.end);
        for (const c of r.list) {
          this.chunks.set(c.hash, got.then(async ({ bytes }) => {
            const stored = bytes.slice(c.offset - r.start, c.offset - r.start + c.length);
            const content = this.check(c, stored, what);
            if (!local) await keep(c.hash, stored); // (kept before it counts as read)
            return content;
          }).catch((e) => { this.chunks.delete(c.hash); throw e; }));
        }
        return got.then(() => Promise.all(r.list.map((c) => this.chunks.get(c.hash))));
      });
    }
    // A file entry's content: its chunks, as kept or fetched.
    async read(e) {
      const kept = await keptHashes();
      const missing = [];
      for (const c of e.chunks) {
        if (this.chunks.has(c.hash)) continue;
        if (kept.has(c.hash)) {
          this.chunks.set(c.hash, caches.match(chunkURL(c.hash), { cacheName: CHUNKS })
            .then(async (r) => {
              if (!r) throw new Error("gone");
              const content = this.check(c, new Uint8Array(await r.arrayBuffer()), e.path);
              stats.stored += c.size;
              return content;
            })
            .catch(async () => { // (evicted or damaged: from the network, then)
              kept.delete(c.hash);
              this.chunks.delete(c.hash);
              await Promise.all(this.fetchRuns([c], e.path));
              return this.chunks.get(c.hash);
            }));
        } else {
          missing.push(c);
        }
      }
      this.fetchRuns(missing, e.path);
      const parts = await Promise.all(e.chunks.map((c) => this.chunks.get(c.hash)));
      const all = new Uint8Array(e.size);
      let at = 0;
      for (const p of parts) { all.set(p, at); at += p.length; }
      return all;
    }
  }

  async function keep(hash, stored) {
    try {
      const c = await caches.open(CHUNKS);
      await c.put(chunkURL(hash), new Response(stored));
      (await keptHashes()).add(hash);
    } catch (err) {
      console.warn("[odi] couldn't keep a chunk (storage full?)", err);
    }
  }

  // A file and its parts as one tree (SPEC.md §5.9): main, runtime and overlay
  // parts merged in order, a later file replacing an earlier one; others
  // under parts/NAME/; the file's own entries on top.
  async function assemble(src, partsBase, depth = 0) {
    if (depth > 8) throw new Error("sets inside sets more than 8 deep");
    const set = await File.open(src);
    const tree = new Map();
    const replaced = [];
    const put = (path, node, by) => {
      const old = tree.get(path);
      if (old && !(old.e.type === "dir" && node.e.type === "dir")) {
        if (old.by !== by) replaced.push({ path, was: old.by, by });
        if (old.e.type === "dir") for (const k of [...tree.keys()]) if (k.startsWith(path + "/")) tree.delete(k);
      }
      tree.set(path, { ...node, by });
    };
    const parts = set.manifest.parts || [];
    const opened = await Promise.all(parts.map((p) => assemble(new URL(p.digest.replace("sha256:", "") + ".odi", partsBase).href, partsBase, depth + 1)));
    parts.forEach((p, i) => {
      const prefix = p.as ? "" : `parts/${p.name}/`;
      if (prefix) {
        put("parts", { e: { path: "parts", type: "dir" } }, "(set)");
        put(prefix.slice(0, -1), { e: { path: prefix.slice(0, -1), type: "dir" } }, "(set)");
      }
      for (const [path, node] of opened[i].tree) put(prefix + path, node, p.name);
    });
    for (const e of set.entries) put(e.path, { e, file: set }, "(set)");
    return {
      digest: set.digest, manifest: set.manifest, signatures: set.signatures, tree, replaced,
      parts: parts.map((p, i) => ({ name: p.name, as: p.as || "", digest: p.digest, size: opened[i].size })),
      size: set.size + opened.reduce((n, o) => n + o.size, 0),
      offline: !!set.offline || opened.some((o) => o.offline),
    };
  }

  // A URL path to a file in the tree, by the manifest's static folders.
  function resolve(app, path) {
    const statics = (app.manifest.static || [{ from: "static", at: "/" }]).slice().sort((a, b) => b.at.length - a.at.length);
    for (const s of statics) {
      const at = s.at.endsWith("/") ? s.at : s.at + "/";
      if (!(path + "/").startsWith(at) && path !== s.at) continue;
      let rel = path.slice(Math.min(path.length, at.length));
      let p = rel ? `${s.from}/${rel}` : s.from;
      let n = app.tree.get(p);
      if (n && n.e.type === "dir") { p += "/index.html"; n = app.tree.get(p); }
      if (!n && !rel.includes(".")) { n = app.tree.get(p + ".html"); }
      if (n && n.e.type === "symlink") {
        const target = new URL(n.e.target, "http://x/" + p).pathname.slice(1);
        n = app.tree.get(target);
      }
      if (n && n.e.type === "file") return n;
    }
    return null;
  }

  const types = {
    html: "text/html; charset=utf-8", js: "text/javascript", mjs: "text/javascript", css: "text/css",
    json: "application/json", wasm: "application/wasm", svg: "image/svg+xml", png: "image/png",
    jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif", webp: "image/webp", ico: "image/x-icon",
    wav: "audio/wav", ogg: "audio/ogg", mp3: "audio/mpeg", woff2: "font/woff2", txt: "text/plain; charset=utf-8",
    pck: "application/octet-stream", wgsl: "text/wgsl; charset=utf-8", glb: "model/gltf-binary",
    gltf: "model/gltf+json", ktx2: "image/ktx2", avif: "image/avif", flac: "audio/flac", opus: "audio/ogg",
  };
  const type = (path) => types[path.split(".").pop().toLowerCase()] || "application/octet-stream";

  // An app's chunks by file, each once.
  function chunksOf(app) {
    const by = new Map();
    for (const n of app.tree.values()) {
      if (n.e.type !== "file" || !n.file) continue;
      if (!by.has(n.file)) by.set(n.file, new Map());
      for (const c of n.e.chunks) by.get(n.file).set(c.hash, c);
    }
    return by;
  }

  // How much of an app is kept here: content bytes, of all.
  async function status(app) {
    const kept = await keptHashes();
    let have = 0, total = 0;
    for (const [file, cs] of chunksOf(app)) {
      for (const c of cs.values()) {
        total += c.size;
        if (isLocal(file.url) || kept.has(c.hash)) have += c.size;
      }
    }
    return { have, total };
  }

  // Everything an app has, fetched ahead and kept (4 runs at a time, up to
  // 4 MB each, neighbours up to 64 KB apart in one request).
  async function save(app, progress) {
    const kept = await keptHashes();
    const tasks = [];
    for (const [file, cs] of chunksOf(app)) {
      if (isLocal(file.url)) continue;
      const want = [...cs.values()].filter((c) => !kept.has(c.hash)).sort((a, b) => a.offset - b.offset);
      for (let i = 0; i < want.length;) {
        let j = i, n = 0;
        while (j < want.length && (j === i || n + want[j].length <= 4 << 20)) n += want[j++].length;
        const group = want.slice(i, j);
        tasks.push(() => Promise.all(file.fetchRuns(group, file.url, 65536, 4 << 20)));
        i = j;
      }
    }
    let next = 0, done = 0;
    const worker = async () => {
      while (next < tasks.length) {
        await tasks[next++]();
        if (progress) progress(++done, tasks.length);
      }
    };
    await Promise.all([worker(), worker(), worker(), worker()]);
    return { requests: tasks.length };
  }

  // The files kept for offline: what each is.
  async function kept() {
    const c = await caches.open(FILES);
    const out = [];
    for (const r of await c.keys()) {
      try {
        const { size, info } = await (await c.match(r)).json();
        out.push({ src: r.url, local: isLocal(r.url), size, name: info.manifest.name, version: info.manifest.version || "", digest: info.digest });
      } catch {}
    }
    return out;
  }

  return { load, assemble, resolve, type, stats, status, save, kept, base, LOCAL, SHELL };
})();
