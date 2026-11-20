# ODI

**One file for a program's build output, any language, that runs anywhere.**

An ODI file (`.odi`) holds what your build produced, exactly as it
produced it, plus a small manifest that says how to start it. The same
file runs on a laptop, a server, a microVM, a container or a serverless
host, and in a browser tab if it's a browser app. Nothing is converted or
recompiled.

```sh
odi build                 # this project -> web.odi
odi run web.odi           # run it here
odi push acme/web:1.4     # publish it (only what the registry lacks is sent)
odi run acme/web:1.4      # run it anywhere, read as it starts (Linux)
```

[![ci](https://github.com/manoj1604/odi/actions/workflows/ci.yml/badge.svg)](https://github.com/manoj1604/odi/actions/workflows/ci.yml)
[Apache-2.0](LICENSE) · status: **0.1, early**. The format may still
change before 1.0, but files built today will keep opening.

## Why

- **Fast cold starts.** An ODI file is cut into checksummed chunks with an
  index, so a runner can start a program after fetching only what it reads
  while starting. Record that once (`odi profile`) and it arrives in one
  request. In our benchmarks, a Next.js app run from a registry 50 ms away
  answered its first request in 0.3–0.6 s instead of 23 s.
- **One artifact everywhere.** One file with one SHA-256 digest, from your
  laptop to CI to production. Sign it with `odi sign`, and runners can
  refuse anything you didn't sign.
- **Parts that snap together.** A *set* is a tiny ODI file that pins other
  ODI files by digest: your app, a shared Node.js runtime, a theme or mod
  layered over it, services it talks to. One runtime part serves every app
  that uses it, stored once.
- **Tools for AI agents.** `odi mcp acme/pdf:1` runs a packaged tool as a
  stdio MCP server, and parts can be attached to a running set.
- **Browser apps and games.** `odi web game.odi` plays a WebAssembly or
  WebGPU game straight out of the file in a browser tab, fetching assets as
  the game asks for them, and can keep it for offline play.

## Install

```sh
curl -fsSL https://github.com/manoj1604/odi/releases/latest/download/install.sh | sh
```

The script picks the build for your machine (Linux or macOS, x86_64 or
arm64), checks it against the release's `SHA256SUMS` and puts `odi` in
`~/.local/bin`. You can also download a build from
[Releases](https://github.com/manoj1604/odi/releases), or build from
source with a recent stable Rust:

```sh
cargo install --git https://github.com/manoj1604/odi odi
```

On Linux, running files from a registry as they're read uses FUSE: install
`fuse3` (`apt install fuse3`). Everything else works without it.

## Quick start

```sh
cd my-app                 # Go, Node.js, Next.js, Python, or a static site
odi build                 # -> my-app.odi
odi inspect my-app.odi    # processes, static files, runtime, size, digest
odi run my-app.odi        # serves on http://localhost:8080
```

`odi build` uses the project's own tools (`go build`, `npm ci` and
`npm run build`, `pip`). A `Procfile` `web:` line, or an `odi.json` with
any manifest fields, overrides what it finds. The same commit always
builds to the same digest.

More in [docs/getting-started.md](docs/getting-started.md).

## What works today

| | macOS | Linux |
|---|---|---|
| `build`, `pack`, `inspect`, `verify`, `extract`, `sign` | yes | yes |
| `run`, `exec`, `mcp` with a local file | yes | yes |
| `run` from a registry | fetched whole first | read as it starts (FUSE) |
| `profile` (record a start list) | no | yes |
| `web` (browser runner) | yes | yes |
| Cross-builds (`--platform`) | Go, Node.js, Next.js | Go, Node.js, Next.js |

Windows isn't supported yet.

## Documentation

- [Getting started](docs/getting-started.md): build, run, publish.
- [Command reference](docs/cli.md): every command and flag (also
  `odi help COMMAND`).
- [Parts and sets](docs/sets.md): runtimes as parts, overlays, services.
- [Tools for AI agents](docs/agents.md): `odi exec`, `odi mcp`, plugs,
  `odi attach`.
- [Browser apps and games](docs/browser.md): `odi web`, hosting the
  runner, offline play.
- [Signing and trust](docs/signing.md).
- [Publishing on the hub](docs/hub.md): versions, ranges, rolling back,
  CI.
- [Registries](docs/registry.md): push, pull, run from a registry, fast
  starts.
- [Configuration](docs/configuration.md): files and environment
  variables.
- [The format specification](SPEC.md): for anyone writing a reader,
  writer, runner or registry.

## Repository

- [`crates/format`](crates/format) (`odi-format`): a Rust library that
  reads and writes ODI files, with every check a reader must make.
- [`crates/odi`](crates/odi): the `odi` command.
- [`crates/web`](crates/web) and [`web/`](web): the browser runner (the
  reader compiled to WebAssembly, plus a page and service worker).
- [`examples/starseed`](examples/starseed): a small WebGPU game packed in
  one ODI file.
- [`bench/`](bench): timing runs from a (mock) registry.

## Contributing

Issues and pull requests are welcome: see [CONTRIBUTING.md](CONTRIBUTING.md).
To report a security problem, see [SECURITY.md](SECURITY.md).

## Licence

[Apache License 2.0](LICENSE).
