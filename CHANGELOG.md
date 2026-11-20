# Changelog

All notable changes to odi are listed here. The project follows
[Semantic Versioning](https://semver.org/); before 1.0, a minor version may
change commands or the 0.2 draft parts of the format.

## [Unreleased]

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
