# Getting started

This walks through building a project into an ODI file, running it, and
publishing it. For every flag, see the [command reference](cli.md) or
`odi help COMMAND`.

## Install

```sh
curl -fsSL https://github.com/manoj1604/odi/releases/latest/download/install.sh | sh
odi version
```

Or build from source: `cargo install --git https://github.com/manoj1604/odi odi`.

## Build

From a project's folder:

```sh
odi build
```

odi works out what the project is and runs its own build:

| Project | Found by | What odi does |
|---|---|---|
| Go | `go.mod` | `go build` with `CGO_ENABLED=0`: one static executable |
| Node.js | `package.json` | `npm ci`, `npm run build` if there's a build script, production `node_modules` |
| Next.js | `next` in `package.json` | as Node.js; with `output: "standalone"` the standalone server, `.next/static` and `public/` served as static files |
| Python | `requirements.txt`, `pyproject.toml`, `app.py` or `main.py` | `pip install` into the app (Flask, FastAPI, Django) |
| Static site | `index.html`, or a `package.json` build that makes `dist/`, `build/` or `out/` | the files, served at `/` |

It writes `NAME.odi` (or `-o FILE`) and prints its digest. Building the
same commit again gives the same digest: file times come from the commit.

To change what odi finds:

- A `Procfile` with a `web:` line sets the web process's command.
- An `odi.json` in the project is merged into the manifest, so it can set
  any manifest field: `{"name": "shop", "env": {"required": ["DATABASE_URL"]}}`.

To build for another machine, add `--platform`, for example
`odi build --platform linux/amd64` on a Mac. Go, Node.js and Next.js
projects are supported: compiled npm packages are swapped for the target's
at the lockfile's versions, and a package that only compiles when
installed (such as `better-sqlite3`) stops the build by name. Python builds
for the machine it runs on.

Build output odi doesn't know how to build can be packed directly. Lay it
out in a folder (`app/` for the program, `static/` for files served as
they are, optionally `runtime/` and `meta/`), write an
`odi.manifest.json`, and run `odi pack DIR`. The manifest is described in
the [specification](../SPEC.md#5-manifest).

## Look inside

```sh
odi inspect shop.odi
odi verify shop.odi       # every byte checked against the digest
odi extract shop.odi out/ # unpack it
```

## Run

```sh
odi run shop.odi
odi run shop.odi --port 3000 -e DATABASE_URL=postgres://localhost/shop
```

odi serves the static files itself and sends every other request to the
`web` process, which gets its port in `PORT`. Each process also gets:

| Variable | Value |
|---|---|
| `ODI_ROOT` | where the file's contents are |
| `ODI_DIGEST` | the file's digest |
| `TMPDIR` | a scratch folder, removed when it stops |

The contents are read-only. Folders the manifest lists under `writable`
are writable scratch space.

A command-line tool runs in the terminal with `odi exec`:

```sh
odi exec tool.odi -- --input report.csv
```

## Publish and run from a registry

```sh
odi login                       # asks for a token
odi push acme/shop:1.0          # only what the registry doesn't have is sent
odi pull acme/shop:1.0          # the build for this machine, checked
odi run acme/shop:1.0           # run it straight from the registry
```

On Linux, `odi run NAME` mounts the file and reads it as the program asks
for it, so a large app starts without downloading all of it. Make it
faster still by recording what it reads while starting:

```sh
odi profile shop.odi --path / --path /products   # Linux
odi push acme/shop:1.0 shop.odi
```

The registry then sends all of that in one request. See
[registries](registry.md) for more, including `odi fill` for hosts that
know what they'll run.

## Next

- [Parts and sets](sets.md): share one runtime between apps, layer a
  theme or mod over an app, run services beside it.
- [Tools for AI agents](agents.md): package a tool and give it to an
  agent as an MCP server.
- [Browser apps and games](browser.md).
- [Signing and trust](signing.md).
