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
odi init               # writes odi.json: what the project is
odi build              # my-app/  ->  my-app.odi
odi run my-app.odi     # runs it
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
odi init                   # write its odi.json (name, type, runtime)
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
- 🔁 **Same input, same file.** The same commit always builds to the same
  file, byte for byte.
- 🏷️ **Versions that make sense.** Publish `1.4.2`, run `:1.4` to get the
  newest patch.
- 📂 **Look inside with `cd`.** `cd app.odi`, then `ls` and `cat` as usual.
- 🚀 **Runs where odi isn't installed.** `odi bundle` makes one executable
  for a VM, a microVM or a server.
- ⚡ **Fast starts.** On Linux, a program run from the hub starts before the
  whole file has downloaded.

## Commands

| You want to… | Run |
|---|---|
| Set up a project | `odi init` (writes [odi.json](docs/odi-json.md)) |
| Build it | `odi build` |
| Run it | `odi run app.odi` |
| See what's inside | `odi inspect app.odi` |
| `cd` into it | `cd app.odi` (once: `eval "$(odi hook zsh)"` in `~/.zshrc`) |
| Run it where odi isn't installed | `odi bundle app.odi -o app`, then `./app` |
| Publish a version | `odi login`, then `odi push you/app:1.0.0` |
| Publish the next patch | `odi push --bump patch` |
| Run it from the hub | `odi run you/app:1` |
| List versions | `odi versions you/app` |
| Get help | `odi help` or `odi help COMMAND` |

That's all most people need. More is in the works (parts and shared
runtimes, tools for AI agents, browser games, signing): `odi help --all`
lists those experimental commands, and [TODO.md](TODO.md) what's planned.

## Learn more

- **[Getting started](docs/getting-started.md)**: a short walkthrough.
- **[Publishing on the hub](docs/hub.md)**: versions, ranges and CI.
- **[Command reference](docs/cli.md)**: every command and flag.

## Status

odi is **0.1, early**. It runs on Linux and macOS; Windows isn't supported
yet. The format may change before 1.0, but files built today will keep
opening.

## Contributing

Issues and pull requests are welcome: see [CONTRIBUTING.md](CONTRIBUTING.md).
To report a security problem, see [SECURITY.md](SECURITY.md).

Licensed under [Apache 2.0](LICENSE).
