# ODI format, version 0.1 (draft), with 0.2 draft additions

> Version 0.1 is what every reader handles. Sections marked **0.2 draft**
> (parts and sets, plugs, runner hints) are implemented by `odi` and may
> still change; a 0.1 reader refuses a set rather than misreading it,
> because a set's manifest says `"odi": "0.2"`.

> Files in this format have the extension **`.odi`**.

An ODI file is **one file that holds a program's build output, as the build
produced it, for any language**, with a manifest that says how to run it,
and, if wanted, the language runtime it runs on. The same file runs on a
laptop, a server, a microVM, in a container or a serverless platform: a
**runner** on each opens the file and starts the program.

It is to build output what a `.jar` is to Java classes and an image is to a
container: one file, one checksum, nothing converted or recompiled.

The key words MUST, SHOULD and MAY are used as in RFC 2119.

## 1. Goals and non-goals

Goals:

- **Any language**: Node.js, Bun, Deno, Python, Java, Go, Rust, Ruby, PHP,
  .NET, static sites. An ODI file holds files; the language only decides
  which files and how they start.
- **One file**: copy, upload, keep, sign and roll back as one unit.
- **Read without unpacking**: a reader finds any file inside with two
  small reads (§3), from disk or over HTTP range requests, so a runner can
  serve an ODI file lazily, fetching only what the program reads.
- **Same content, same identity**: the ODI file's digest (§6) depends on
  the files and the manifest, not on how they were compressed, so
  identical builds have identical digests.
- **Deploy anywhere**: the manifest says what the program needs (a
  command, a port, static files, environment variable names), never where
  it runs.

Non-goals:

- Not a compiler: output isn't transformed into a native executable.
- Not an operating-system image: an ODI file carries no kernel and no OS
  distribution. Native code inside targets a platform (§5.2).
- Not a secrets store: an ODI file names the environment variables it needs
  and MUST NOT contain their values.

## 2. Terms

- **ODI file**: a file in this format.
- **Builder**: a tool that makes an ODI file from a project.
- **Runner**: a tool that runs an ODI file (locally, or on a platform).
- **Entry**: one file, directory or symbolic link inside an ODI file.
- **Chunk**: a piece of an entry's content, stored compressed.

## 3. File layout

```
offset 0  ┌──────────────────────────────┐
          │ Header (64 bytes)            │
          ├──────────────────────────────┤
          │ Chunk data                   │  compressed chunks, any order
          ├──────────────────────────────┤
          │ Manifest (JSON, zstd)        │
          ├──────────────────────────────┤
          │ Index (zstd)                 │
          ├──────────────────────────────┤
          │ Signatures (optional)        │
          ├──────────────────────────────┤
          │ Footer (128 bytes)           │
   EOF    └──────────────────────────────┘
```

The index and manifest come **after** the data, so a builder can write a
ODI file in one pass, and the **footer has a fixed size at the end**, so a
reader that knows the file's length reads the footer, then the manifest
and index, then only the chunks it needs.

All integers are little-endian. Offsets are from the start of the file.

### 3.1 Header (64 bytes)

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | magic: `ODIFILE\0` (ASCII, then a zero byte) |
| 8 | 2 | format major version (0 for this draft) |
| 10 | 2 | format minor version (1) |
| 12 | 4 | flags (reserved: 0) |
| 16 | 48 | reserved: zero |

### 3.2 Footer (128 bytes, the last bytes of the file)

| Offset in footer | Size | Field |
|---|---|---|
| 0 | 8 | manifest offset |
| 8 | 8 | manifest length (stored bytes) |
| 16 | 8 | index offset |
| 24 | 8 | index length (stored bytes) |
| 32 | 8 | signatures offset (0: none) |
| 40 | 8 | signatures length |
| 48 | 32 | ODI digest (§6), raw SHA-256 |
| 80 | 40 | reserved: zero |
| 120 | 8 | magic: `ODIFILE\0` |

A reader MUST check both magics and that every offset and length lies
within the file. A reader MUST refuse a major version it doesn't know and
SHOULD accept a higher minor version of a major it knows (minor versions
only add optional fields).

### 3.3 Chunk data

An entry's content is cut into chunks of at most **4 MiB** (uncompressed).
Builders SHOULD cut at content-defined boundaries (a rolling hash), so a
small change in a big file changes few chunks; fixed sizes are allowed.

Each chunk is stored as:

| Size | Field |
|---|---|
| 1 | encoding: 0 = stored as is, 1 = zstd |
| 3 | reserved: zero |
| n | the (compressed) bytes |

A chunk that occurs several times (in one entry or in several) MAY be
stored once and referenced from each place. A chunk's identity is the
SHA-256 of its **uncompressed** bytes.

### 3.4 Index

The index is a JSON document, compressed with zstd. It lists every entry,
sorted by path (byte order):

```json
{
  "entries": [
    { "path": "app",                  "type": "dir",     "mode": 493 },
    { "path": "app/server.js",        "type": "file",    "mode": 420, "size": 18230,
      "chunks": [ { "hash": "9f2c…", "offset": 64, "length": 6110, "size": 18230 } ] },
    { "path": "app/node_modules/.bin/next", "type": "symlink", "target": "../next/dist/bin/next" }
  ]
}
```

- `path`: relative, `/`-separated, no `.` or `..` component, no leading
  `/`. Readers MUST refuse an ODI file with any other path.
- `type`: `file`, `dir` or `symlink`. A symlink's `target` is relative
  and MUST NOT point outside the ODI file, followed as a filesystem would:
  through any links on its way, at most 40. Nothing is stored under a file
  or a symlink. Readers MUST refuse an ODI file that breaks either rule.
- `mode`: permission bits only (0o777 at most; no setuid, setgid or sticky).
- `size`: a file's uncompressed size; its chunks, in order, hold exactly
  that many bytes.
- `chunks[]`: `hash` (hex SHA-256 of the uncompressed chunk), `offset` and
  `length` of the stored chunk (§3.3) in the file, `size` uncompressed.

Ownership, timestamps and extended attributes are not recorded: a runner
gives every file a fixed time, so identical content stays identical.

### 3.5 Signatures (optional)

A JSON document listing signatures of the ODI digest, each with an
algorithm (`ed25519` in this version), the public key or its identifier,
and the signature. Signatures are outside the digest, so an ODI file can be
signed after it's built.

```json
{ "signatures": [
  { "alg": "ed25519",
    "key": "ed25519:93073bbf…74",
    "sig": "5f1c…(128 hex digits)" } ] }
```

- Compressed with zstd, like the manifest and index; at most 64 KiB once
  decompressed. Builders put it after everything else, just before the
  footer, so signing again replaces it and keeps every other byte.
- `key`: `ed25519:` and the public key's 32 bytes in hex.
- `sig`: the ed25519 signature (RFC 8032, checked strictly) of the bytes
  `odi-signature-v0\n` followed by the 32 bytes of the ODI digest.
- A runner that trusts keys runs a file only if a signature by one of them
  checks. A set (0.2) is signed like any file: its digest pins its parts'.

## 4. Contents: the layout inside

Builders MUST put files under these top-level directories, and nothing
else at the top:

| Directory | Holds |
|---|---|
| `app/` | the program: build output and its production dependencies, laid out as it runs (the working directory, unless the manifest says otherwise) |
| `static/` | files served as they are, over HTTP (optional) |
| `runtime/` | the language runtime, when included (optional; §5.3) |
| `meta/` | anything else a builder wants to keep: licences, an SBOM, build logs (optional; not used to run) |

Examples (informative):

| Project | `app/` | `static/` | `runtime/` (when included) |
|---|---|---|---|
| Next.js | `.next/standalone` contents, production `node_modules`, `public/` (its image optimizer reads originals from its own server) | `.next/static` (at `/_next/static`), `public/` | Node.js |
| Express, Fastify | the source or compiled JS, production `node_modules` | optional | Node.js or Bun |
| FastAPI, Django | the app, its installed packages (`site-packages`) | collected static files | CPython |
| Spring Boot | the `.jar` | | a JRE |
| Go, Rust | the built executable, any files it reads | | none |
| Rails | the app, its gems | `public/assets` | Ruby |
| Vite, Astro (static) | empty | the built site | none |

## 5. Manifest

The manifest is a JSON document, compressed with zstd. Fields not listed
here MUST be ignored by readers (§3.2).

```json
{
  "odi": "0.1",
  "name": "acme-web",
  "version": "2026.10.23-3f2a91c",
  "platform": { "os": "linux", "arch": "amd64", "libc": "gnu" },
  "language": { "name": "node", "framework": "nextjs" },
  "runtime": {
    "name": "node",
    "version": "24.22.0",
    "included": false,
    "digest": "sha256:5c1e…"
  },
  "processes": [
    {
      "name": "web",
      "command": ["node", "server.js"],
      "workdir": "app",
      "port": { "env": "PORT", "default": 3000, "protocol": "http" },
      "health": { "path": "/api/health" }
    }
  ],
  "static": [
    { "from": "static/_next/static", "at": "/_next/static", "immutable": true },
    { "from": "static/public", "at": "/" }
  ],
  "env": {
    "required": ["DATABASE_URL", "AUTH_SECRET"],
    "optional": ["LOG_LEVEL"],
    "defaults": { "NODE_ENV": "production" }
  },
  "resources": { "memory": "512MiB" },
  "build": {
    "source": "https://example.com/acme/web.git",
    "revision": "3f2a91c4e7…",
    "builder": "odi-build 0.1.0",
    "time": "2026-10-23T10:00:00Z"
  }
}
```

### 5.1 Identity

- `odi` (required): the manifest's version, `"0.1"`.
- `name` (required): letters, digits, `.`, `-`, `_`; at most 128.
- `version` (optional): free text, at most 128 characters.

### 5.2 Platform

`platform` (required when the ODI file holds native code, which most do):

- `os`: `linux`, `darwin`, `windows`, or `any` (pure scripts and static
  sites).
- `arch`: `amd64`, `arm64`, or `any`.
- `libc` (Linux): `gnu`, `musl`, or `none` (static).

A runner MUST refuse an ODI file for another platform unless the platform is
`any` (or it can provide that platform, e.g. by emulation, and says so).

### 5.3 Runtime

`runtime` (optional; absent: the program needs none):

- `name`: `node`, `bun`, `deno`, `python`, `java`, `ruby`, `php`,
  `dotnet`, or a name a runner knows.
- `version`: exact (`"24.22.0"`) or a range (`">=22 <25"`, `"3.13"`).
- `included`: `true` when the runtime is in `runtime/`. Its executables
  are in `runtime/bin/`, which a runner puts first on `PATH`.
- `digest`: when not included, the SHA-256 of the runtime a runner should
  fetch (a runtime from a catalogue it trusts, or the language's own
  release), so every runner runs the same one.

A runner MUST provide the runtime (the included one, or one matching
`version` and `digest`) or refuse to run.

### 5.4 Processes

`processes` (one or more; MAY be absent when the ODI file only serves
`static` files, e.g. a static site): what to start. Each has:

- `name`: unique; `web` is the conventional name of the HTTP process.
- `command`: an array, the program first. A relative program name is
  looked up in `runtime/bin/` (when there's a runtime), then `app/`, then
  the runner's `PATH`.
- `workdir`: a directory in the ODI file (default `app`).
- `port` (optional): the process listens for `protocol` (`http`, `tcp`)
  on the port the runner passes in the environment variable `env`
  (default `PORT`); `default` when the runner passes none.
- `health` (optional): an HTTP path that answers 2xx when ready.

### 5.5 Static files

`static` (optional): directories of the ODI file served as they are at a URL
path; `immutable: true` lets a runner or CDN cache them forever. A
platform MAY serve them from a CDN and send other requests to `web`.

### 5.6 Environment

`env.required` and `env.optional` name variables the program reads; the
runner (or the person deploying) gives their values. `env.defaults` gives
values that are not secret. An ODI file MUST NOT contain secrets.

### 5.7 Build

`build` (optional, informative): where it came from and what made it.

### 5.8 Writable folders

`writable` (optional): folders under `app/` the program writes to, such as a
framework's cache. Contents are thrown away when it stops: this is scratch
space, not storage. A runner that serves `app/` read-only (§7) MUST make
each one a writable folder, and entries inside it in the ODI file need not
be shown.

### 5.9 Parts and sets (0.2 draft)

A **set** is an ODI file whose manifest lists other ODI files, its
**parts**, by digest. Its manifest says `"odi": "0.2"` and has a `parts`
list (a set may also have entries and processes of its own):

```json
{
  "odi": "0.2",
  "name": "shop",
  "parts": [
    { "name": "shop", "digest": "sha256:877a…", "as": "main", "from": "acme/shop:1.4" },
    { "name": "node", "digest": "sha256:35d7…", "as": "runtime", "from": "acme/node:22" },
    { "name": "theme", "digest": "sha256:9b02…", "as": "overlay" },
    { "name": "prices", "digest": "sha256:c4e1…" }
  ]
}
```

- `name` (required): unique in the set; letters, digits, `.`, `-`, `_`.
- `digest` (required): the part's ODI digest (§6), `sha256:` and 64 hex
  digits. A runner MUST check that the part it got has this digest.
- `as` (optional): `main`, `runtime` or `overlay`, or absent.
- `from` (optional, informative): where the builder got it, such as a
  registry name. Runners MAY use it to find the part; the digest decides.

A runner puts a set together into one tree before running it:

1. The parts, in order. A `main`, `runtime` or `overlay` part's entries are
   merged at the top of the tree; where two parts have an entry at the same
   path, the later one replaces the earlier (a directory replacing a
   directory merges them). A part with no `as` goes under `parts/NAME/`.
2. Then the set's own entries, on top.

A part may itself be a set; a runner MUST refuse sets nested more than 8
deep. Because the set's digest covers its manifest, which pins every part's
digest, the set's digest (and any signature over it, §3.5) covers every
byte of the assembled tree.

A runner SHOULD say which files one part replaced in another, so an overlay
(a theme, a mod) can be reviewed.

A file's `runtime.digest` (§5.3) MAY name a runtime part: a runner that has
that ODI file merges it as the `runtime` part.

### 5.10 Plugs (0.2 draft)

`provides` and `needs` (optional) say what a file offers to, and wants
from, the rest of a set:

```json
"provides": [{ "plug": "http", "name": "prices", "process": "web" }],
"needs": [{ "plug": "http", "name": "prices" }]
```

- `plug` (required): `http`, `tcp` (a service a process serves on its
  port), `mcp` (a stdio MCP server), `cli` (command-line tools in
  `app/bin/`), or a name a runner knows.
- `name` (required): the plug's name.
- `version` (optional): free text.
- `process` (optional): which process serves it (default: the first).

A runner that runs a set:

- MUST refuse to run it when something in `needs`, of the set or of a part,
  isn't in any part's `provides`, naming what's missing;
- starts each part's `http` and `tcp` services on a port of its own, and
  gives every process `ODI_PLUG_<NAME>_URL` (the name in capitals, other
  characters as `_`) with where each listens;
- puts `cli` parts' `app/bin/` on `PATH`.

### 5.11 Runner hints (0.2 draft)

A builder MAY keep hints under `meta/odi/` that let a runner fetch ahead
what a program will read. A runner MUST run correctly without them, and
MUST ignore a hint that's missing, stale or unreadable.

- `meta/odi/start.json`: what the program read while starting and
  answering its first requests, in order (made by `odi profile`). A runner
  reading from a remote source fetches these chunks first, in as few
  requests as it can.
- `meta/odi/routes.json`: for each route of a web app, the files it needs.

Builders SHOULD write these entries last, next to the index, so a runner
gets them with the index.

## 6. Digest

The **ODI digest** identifies an ODI file's contents:

```
digest = SHA-256( "odi-v0\n" || canonical(manifest) || "\n" || canonical(entries) )
```

where `canonical(x)` is x as JSON with object keys sorted by their bytes
(UTF-8), no insignificant whitespace, strings with only the escapes JSON
requires (`"`, `\`, and control characters as `\n`, `\t`... or `\u00XX`;
`<`, `>`, `&` and non-ASCII characters as they are), and numbers exactly as
they were written; `entries` is the index's entry list
**without** the `offset` and `length` fields (they depend on layout and
compression). So two ODI files with the same files and manifest have the
same digest whatever order or compression their chunks use. It's written
as `sha256:` and 64 hex digits.

A reader MUST recompute the digest from the manifest and index it read and
compare it with the footer's; a reader that reads a chunk MUST check its
hash before using it.

## 7. Running an ODI file (the runner contract)

A runner:

1. Reads the footer, manifest and index; checks the digest; checks the
   platform (§5.2); verifies signatures if its policy asks.
2. Provides the runtime (§5.3).
3. Makes the files available at a root directory: unpacked, or mounted
   read-only and fetched lazily. Programs MUST be able to run from a
   read-only `app/`, apart from the folders named in `writable` (§5.8); a
   runner gives a writable temporary directory in `TMPDIR`.
4. Starts each process in its `workdir` with: `env.defaults`, the values
   given for `env.required`/`optional`, `PORT` (or the named variable),
   `ODI_ROOT` (the root), `ODI_DIGEST`, and `PATH` with
   `runtime/bin` first.
5. Serves `static` itself or hands it to the platform, and sends other
   HTTP requests to `web`.

## 8. Deploying anywhere (informative)

| Where | A runner or deployer does |
|---|---|
| A laptop | runs it from the file |
| A server, VM or microVM | runs it as a service; a microVM can boot with the ODI file mounted |
| Containers (Docker, Kubernetes) | exports an OCI image: a base with the runner, the ODI file as a layer |
| AWS Lambda | packages `app/` (+ runtime) for a function, `static/` to object storage and a CDN |
| Cloudflare | `static/` to object storage and the edge; `web` where a runtime is available |
| Hosting platforms | reads the manifest to route static files and requests |

## 9. Security considerations

- **Paths**: §3.4's rules stop an ODI file writing outside its root when
  unpacked; runners MUST enforce them, not trust them. A disk can join names
  the index keeps apart (`K` and `k`), so unpacking MUST also refuse to write
  through a link and check where links lead once they exist.
- **Running**: an ODI file is a program. A runner that starts one without
  isolation (as `odi run` does) gives it what its user has: only run files
  you trust, or run them in a sandbox.
- **Integrity**: the digest covers the manifest and every file's content
  hashes; chunks are checked as read.
- **Authenticity**: the digest proves contents, not origin; use signatures
  (§3.5) and pin trusted keys.
- **Secrets**: never in an ODI file; required variables are named only.
- **Runtimes fetched by digest** (§5.3) can't be swapped.
- **Resource limits**: a reader SHOULD cap the index's and manifest's
  decompressed sizes and the number of entries it accepts, and MUST NOT
  decompress a chunk past `MaxChunk`.

## 10. Media type and extension

`application/vnd.odi.v0`; extension `.odi`.

## Implementations

- [`odi`](https://github.com/manoj1604/odi) (Rust; the library is
  `odi-format`): builds (Go, Node.js, Next.js, Python, static sites),
  packs, inspects, verifies, signs and runs ODI files, sets included, from
  a file or a registry. It cuts chunks with a gear rolling hash between
  512 KiB and 4 MiB (about 1 MiB), and writes runner hints (§5.11).
- The browser runner in the same repository (`web/`, `crates/web`): the
  reader's checks compiled to WebAssembly, running browser apps out of an
  ODI file with HTTP Range requests.

## Open questions

- Content-defined chunking parameters, so independent builders produce the
  same chunks (better dedup across tools).
- Settling sets, plugs and runner hints (§5.9–5.11) for 0.2.
- Shared compression dictionaries, and small files sharing a chunk, so
  many small text files compress about as well as in one stream.
- Windows paths and permissions.
- A standard list of runtime names and how their versions are matched.
