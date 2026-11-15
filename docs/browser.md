# Browser apps and games

ODI files of browser apps (anything whose files are under `static/`: a
static site, a canvas or WebGPU game, a Godot, Unity or Emscripten export)
run straight out of the file in a browser tab. The tab fetches only the
parts of the file the app asks for, checks every byte, and can keep the app
for offline use.

```sh
odi build ./my-game       # a folder with index.html is a static site
odi web my-game.odi       # prints http://localhost:8080/?src=files/my-game.odi
```

## How it works

The browser runner is four static files (`web/` in this repository, also
built into `odi`):

| File | What it is |
|---|---|
| `index.html` | the page: opens a file, shows what it is, runs the app in a frame |
| `sw.js` | a service worker that answers the app's requests out of the ODI file |
| `odi.js` | the reader: Range requests, assembling sets, keeping things for offline |
| `odi_web.wasm` | the reader's checks (manifest, index, digest, signatures, each chunk), the same Rust code `odi` uses, compiled to WebAssembly |

When the app asks for `game.js` or `level2.ogg`, the service worker looks it
up in the file's index, fetches only that file's chunks with an HTTP Range
request, checks and decompresses them, and answers. Music for a level the
player hasn't reached isn't downloaded.

Sets work as with `odi run`: a mod is an overlay part, and the runner shows
how many files it replaced. Signatures are checked and shown.

Pages run cross-origin isolated (COOP/COEP headers), so apps that need
`SharedArrayBuffer` and threads can have them. WebGPU needs a browser that
supports it (Chrome, Edge, Safari 26 or later) and HTTPS or localhost.

## Hosting

Any static host or CDN that supports Range requests can serve the runner
and ODI files: put the four files in a folder (each release has them as
`odi-web-VERSION.tar.gz`), and open `index.html?src=URL-OF-AN-ODI-FILE`.
For a set, `parts=URL` names the folder its parts are in, as
`DIGEST.odi` files (default `parts/`). Serve the runner's pages with these
headers for cross-origin isolation:

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

## Offline

- Every chunk the tab reads is kept in the browser by its hash, still
  compressed, and checked again each time it's used. A new version of the
  app downloads only the chunks that changed.
- **Save for offline** in the runner's bar downloads the rest of the app
  ahead of time.
- The runner keeps its own files, so with no network it opens, finds the
  app's kept index, and runs it.
- Opening the runner with no `src` shows a box to pick or drop a `.odi`
  file from your computer (nothing is uploaded), and a list of apps opened
  before.

## Example

[`examples/starseed`](../examples/starseed) is a small WebGPU game: a Rust
core compiled to WebAssembly, two WGSL shaders and generated music, in one
3.9 MB ODI file. In our tests in Chrome, its first frame came 94 ms after
the page opened, with 91 KB of the file fetched.

## What the app should know

- Paths are relative to the app's folder: the runner serves it at
  `run/KEY/`, not at `/`. Use relative URLs (`game.js`, not `/game.js`).
- To tell the runner when it's ready (shown in its timings), post a
  message to the parent: `parent.postMessage({ odi: "ready", what: "first frame" }, "*")`.
- Server programs can't run in a tab: they need `odi run`.
