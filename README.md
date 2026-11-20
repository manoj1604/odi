<p align="center">
  <img src="docs/assets/odi-icon.svg" width="112" alt="ODI icon">
</p>

<h1 align="center">odi</h1>

<p align="center">
  <b>Put your app in one file. Run it anywhere.</b>
</p>

<p align="center">
  <a href="https://github.com/manoj1604/odi/actions/workflows/ci.yml"><img src="https://github.com/manoj1604/odi/actions/workflows/ci.yml/badge.svg" alt="ci"></a>
  <a href="https://github.com/manoj1604/odi/releases/latest"><img src="https://img.shields.io/github/v/release/manoj1604/odi" alt="release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="license"></a>
</p>

---

**odi** turns what your build produces into a single `.odi` file. That one
file runs on your laptop, a server, in CI or in a browser tab, exactly
the same, with nothing to convert or rebuild.

```sh
odi build              # my-app/  ->  my-app.odi
odi run my-app.odi     # runs it on http://localhost:8080
```

Works with **Go, Node.js, Next.js, Python and static sites**, using each
project's own build tools.

## Install

```sh
curl -fsSL https://github.com/manoj1604/odi/releases/latest/download/install.sh | sh
```

For Linux and macOS (x86_64 and arm64). Or, with Rust installed:
`cargo install --git https://github.com/manoj1604/odi odi`.

## Try it in a minute

```sh
cd my-app
odi build                  # build it into my-app.odi
odi inspect my-app.odi     # see what's inside
odi run my-app.odi         # run it
```

Share it through the hub:

```sh
odi login
odi push you/my-app:1.0.0  # publish a version
odi run you/my-app:1       # run the newest 1.x anywhere
```

## Why odi?

- 📦 **One file.** Your whole build in one file with one checksum, the
  same from laptop to production.
- ⚡ **Fast starts.** On Linux, a program can start before the whole file has
  downloaded, because only what it reads first is fetched.
- 🔁 **Same input, same file.** The same commit always builds to the same
  file, byte for byte.
- 🔐 **Signed.** `odi sign` signs a file, and you can refuse to run anything
  you didn't sign.
- 🏷️ **Versions that make sense.** Publish `1.4.2`, run `:1.4` to get the
  newest patch, and roll back with one command.
- 🤖 **Tools for AI agents.** `odi mcp you/tool:1` runs a packaged tool as
  an MCP server.
- 🎮 **Browser apps and games.** `odi web game.odi` plays a WebAssembly or
  WebGPU game straight from the file, even offline.

## Common commands

| You want to… | Run |
|---|---|
| Build this project | `odi build` |
| Run a file | `odi run app.odi` |
| See what's inside | `odi inspect app.odi` |
| Run a command-line tool | `odi exec tool.odi -- ARGS` |
| Publish a version | `odi push you/app:1.0.0` |
| Publish the next patch | `odi push --bump patch` |
| List versions | `odi versions you/app` |
| Roll back | `odi tag you/app:1.0.0 latest` |
| Open in a browser | `odi web game.odi` |
| Get help | `odi help` or `odi help COMMAND` |

## Learn more

- **[Getting started](docs/getting-started.md)**: a short walkthrough.
- **[Publishing on the hub](docs/hub.md)**: versions, ranges, rollbacks and CI.
- **[All docs](docs/README.md)**: sets, AI agents, browser games, signing,
  configuration.
- **[Command reference](docs/cli.md)**: every command and flag.

## Status

odi is **0.1, early**. It runs on Linux and macOS; Windows isn't supported
yet. The format may change before 1.0, but files built today will keep
opening.

## Contributing

Issues and pull requests are welcome: see [CONTRIBUTING.md](CONTRIBUTING.md).
To report a security problem, see [SECURITY.md](SECURITY.md).

Licensed under [Apache 2.0](LICENSE).
