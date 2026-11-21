# Changelog

All notable changes to odi are listed here. The project follows
[Semantic Versioning](https://semver.org/); before 1.0, a minor version may
change commands or the 0.2 draft parts of the format.

## [0.2.0] - 2026-11-21

### A small core

- `odi help` shows the few commands most people need: `build`, `run`,
  `bundle`, `init` (odi.json), `inspect`, `hook` (`cd` into a file),
  `close`, and `login`,
  `push`, `pull`, `versions`, `logout`. Every other command works as before
  and is listed by `odi help --all` as experimental; [TODO.md](TODO.md)
  says what's planned. The README and the getting-started guide cover the
  core only.

### odi.json: every project has one

- `odi build` needs an `odi.json` in the project: `name` and `type` (`go`,
  `node`, `nextjs`, `python`, `static`) are required; the rest is optional:
  `version`, `repo`, `runtime` ("node 22"), `include_runtime`, `build`,
  `start`, `port`, `static`, `env` (a default, or "required" /
  "optional"), and `manifest` (raw manifest fields). A key that isn't in the
  specification is an error. Specification: docs/odi-json.md; JSON Schema:
  schema/odi.schema.json.
- `odi init` writes one from what it finds in the project.
- `"include_runtime": true` puts Node.js itself in the file: the official
  release for the target, checked against its published checksums, so the
  app runs where Node.js isn't installed.
- The `cd` hook is `odi hook zsh|bash|fish` (it was briefly `odi init`).
- A Procfile still sets the start command when odi.json doesn't.

### Next.js: the app serves its static files by default

- `odi build` puts `.next/static` and `public/` inside `app/`, where
  Next.js serves them, so `node server.js` serves the whole site, by hand
  too. `odi build --static odi` has odi serve them (as before: cached
  forever, CDN-ready), and keeps the app's copy. A Next.js build's digest
  changes with this.

### Run without odi installed

- `odi bundle FILE -o OUT`: one executable that is the app, a static odi
  for the target machine with the ODI file after it, unchanged. Run it on a
  VM, microVM or server with nothing installed (with `include_runtime`, not
  even Node.js); `OUT --odi-file F` gets the ODI file back. The runner is this odi when it fits, else the release
  for the target, checked against its SHA256SUMS.

### Step inside a file

- `cd app.odi`: `odi hook zsh|bash|fish` prints a `cd` for the shell's
  startup file that goes into `.odi` files (`cd app.odi/app`, and hub names
  with a tag, `cd acme/web:1.4`), read-only, so `ls`, `cat` and the rest
  work inside. Mounted on Linux (read as you look), unpacked once elsewhere.
  `odi close` lets them go.
- `odi shell FILE|NAME [PATH]` opens a shell inside the file, ready to
  start the app by hand (`node server.js`): in its folder, with its runtime
  on PATH, its environment, `PORT` and writable folders, and odi serving its
  static files on the app's port. Mounted read-only on Linux, a private copy
  elsewhere. `-- COMMAND` runs one command there instead.
- `odi login` says where to make a hub API key.
- An unpacked file is written to disk before it's marked done: a machine
  that loses power (or a microVM killed) no longer comes back to a broken
  copy.

### Publishing on the hub

- `odi push` publishes versions: the one named, the next one
  (`--bump patch|minor|major`), or the build's own; a release also moves
  `latest`, a pre-release doesn't. `--tag`, `--no-latest`.
- A published version doesn't change: another build pushed to it is
  refused (`--force` moves it). `--add` adds a platform to a version.
- Names take version ranges everywhere: `acme/web:1.4`, `:1`, `:^1.4`,
  `:~1.4.2`.
- `odi versions` (versions and tags, platforms, sizes, dates), `odi tag`
  (promote or roll back without pushing), `odi diff` (what changed between
  two builds, and what a machine with the old one must fetch),
  `odi search`, `odi repos`.
- A project's `odi.json` can name its repository (`"repo"`), and
  `odi build` takes the version from a git tag, `package.json` or
  `pyproject.toml`.

## [0.1.0] - 2026-11-15

The first public release.

### The format

- ODI 0.1: one file holding a program's build output, cut into
  content-defined, zstd-compressed, SHA-256-checked chunks, with a
  manifest, an index and a digest over both ([SPEC.md](SPEC.md)).
- Signatures (ed25519) kept in the file without changing its digest.
- 0.2 draft: sets (parts pinned by digest, put together when run), plugs
  (`provides`, `needs`), runner hints (start lists, route maps).

### The odi command

- `odi build` for Go, Node.js, Next.js, Python and static sites, with
  reproducible digests and cross-builds (`--platform`) for Go, Node.js and
  Next.js; `odi pack` for any laid-out folder.
- `odi run`, `odi exec`, `odi mcp`: run a file, one of its processes, or
  its MCP server; from a registry, read as it starts (Linux, FUSE).
- `odi set`, `odi runtime`, `odi part`: sets, runtime parts (Node.js from
  nodejs.org, checked), this machine's store of parts.
- `odi profile`, `odi fill`: start lists, and fetching them ahead.
- `odi attach`: add a part to a running set.
- `odi web`: the browser runner, with offline play and local files.
- `odi inspect`, `odi verify`, `odi extract`, `odi key`, `odi sign`.
- `odi login`, `odi push`, `odi pull`, `odi tags`, `odi rm`, `odi usage`:
  registries, sending and fetching only missing pieces.
- `odi help COMMAND` with flags and examples, `odi completions`.

### Also

- `odi-format`, a Rust library to read and write ODI files.
- Builds for Linux (static) and macOS, x86_64 and arm64, and an install
  script.
