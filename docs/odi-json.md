# odi.json

Every project `odi build` builds has an `odi.json` at its top. It says what
the project is and how to build and start it. `odi init` writes one for an
existing project; you can also write it by hand.

```json
{
  "$schema": "https://raw.githubusercontent.com/manoj1604/odi/main/schema/odi.schema.json",
  "name": "shop",
  "type": "nextjs",
  "repo": "acme/shop",
  "runtime": "node 22",
  "include_runtime": true,
  "env": {
    "NODE_ENV": "production",
    "DATABASE_URL": "required"
  }
}
```

This page is the specification: the keys, what each means, what's assumed
when a key is left out, and what makes a file invalid. A JSON Schema of it
is [`schema/odi.schema.json`](../schema/odi.schema.json); editors that know
JSON Schema check the file as you type when `$schema` points to it.

## Keys

Only `name` and `type` are required.

| Key | Type | Required | What it says |
|---|---|---|---|
| `$schema` | string | no | the JSON Schema's address, for editors; odi ignores it |
| `name` | string | **yes** | the app's name: 1 to 64 of `a-z 0-9 . _ -`, starting with a letter or digit |
| `type` | string | **yes** | what kind of project: `go`, `node`, `nextjs`, `python` or `static` |
| `version` | string | no | the build's version, `1.4.2` |
| `repo` | string | no | where `odi push` publishes, `namespace/name` |
| `runtime` | string | no | the runtime and its version: `"node 22"`, `"node 22.11.0"`, `"python 3.12"` |
| `include_runtime` | boolean | no | `true`: the runtime itself goes in the file |
| `build` | string | no | the build command |
| `start` | string | no | the command that starts the app |
| `port` | integer | no | the port the app listens on, 1 to 65535 |
| `static` | string | no | `static` projects: the folder to serve |
| `env` | object | no | environment variables the app gets or needs |
| `manifest` | object | no | raw manifest fields, for what the keys above don't cover |

Any other key makes the file invalid (a misspelled key is an error, not
silently ignored).

### name

The app's name, as `odi inspect` shows it and the file is named
(`NAME.odi`). Lowercase letters, digits, `.`, `_` and `-`, at most 64,
starting with a letter or digit.

### type

| `type` | The project | Default build | Default start |
|---|---|---|---|
| `go` | `go.mod` | `go build` with `CGO_ENABLED=0`: one static executable | the executable |
| `node` | `package.json` | `npm ci`, then `npm run build` if there's a build script; production `node_modules` | `node` with the `start` script's file, else `main`, else `index.js` |
| `nextjs` | `package.json` with `next`, and `output: "standalone"` in `next.config` | `npm ci`, `npm run build` | `node server.js` |
| `python` | `requirements.txt`, `pyproject.toml`, `app.py` or `main.py` | `pip install` into the app | `python3 app.py` (or `main.py`) |
| `static` | files to serve | none, unless `build` is given (or a `package.json` build script) | none: odi serves the folder |

The type must match the project: `type: "go"` with no `go.mod` is an error,
and so is `type: "node"` for a project that depends on `next` (use
`nextjs`).

### version

Free text, usually a version like `1.4.2`. When it's left out: a version tag
on the current commit (`v1.4.2`), else the version in `package.json` or
`pyproject.toml`, else the commit. `odi push` publishes under it when it's
a version.

### repo

`namespace/name` on the hub, such as `acme/shop`: where `odi push` and
`odi versions` go when no name is given. It isn't part of the built file.

### runtime

The runtime's name and version, separated by a space. The name must be the
type's: `node` for `node` and `nextjs`, `python` for `python`; `go` and
`static` projects have none. The version may be exact (`22.11.0`) or a line
(`22`, `22.11`). When it's left out: `.nvmrc`, `.node-version` or
`package.json`'s `engines.node` for Node.js, else the major version of the
`node` on this machine.

### include_runtime

`false` (the default): the file holds the app, and the machine that runs it
needs the runtime. `true`: the runtime itself goes in the file, in
`runtime/`, so the app runs where it isn't installed (`odi run`, `cd` into
the file, `odi bundle`). For Node.js, odi uses the official release from
nodejs.org for the target machine (`--platform`, else this one), checked
against its published SHA-256, and the newest release in the `runtime` line.
It adds about 40 MB to the file. Node.js only for now.

### build

A shell command run in the project's folder, in place of the type's default
build (for `node` and `nextjs`, after `npm ci`). `""` builds nothing.

### start

The command that starts the app, run in its folder inside the file (`app/`)
with `PORT` set. A plain command (`node server.js`) runs as it is; one with
shell syntax (`npm run migrate && node server.js`) runs through `sh -c`.

### port

The port the app listens on unless told otherwise. It's given its port in
`PORT` either way; `odi run --port` changes it. Default: 3000 for `node` and
`nextjs`, 8000 for `python`, 8080 for `go` and `static`.

### static

For `static` projects: the folder to serve, relative to the project.
Default: the first of `dist`, `build` and `out` that has an `index.html`
after the build, else the project itself.

### env

Each key is an environment variable; its value is one of:

- a default value (a string, number or boolean): the app gets it unless the
  person running it gives another (`odi run -e KEY=VALUE`);
- `"required"`: the app must be given it, and `odi run` stops and asks;
- `"optional"`: the app may be given it, and is told nothing otherwise.

### manifest

Raw manifest fields ([SPEC.md §5](../SPEC.md#5-manifest)), merged into what
odi builds, for what the keys above don't cover: more processes,
health checks, writable folders. Most projects don't need it.

## Validity

A file is invalid, and `odi build` stops, when:

- it isn't a JSON object;
- `name` or `type` is missing, or `name` isn't a valid name;
- `type` isn't one of the five, or doesn't match the project;
- a key isn't one of those above;
- a value has the wrong type (`"port": "3000"`), or is out of range;
- `runtime` names another type's runtime;
- `include_runtime` is `true` for a runtime odi can't include yet, or for a
  type with no runtime.

## Examples

A Go service:

```json
{ "name": "api", "type": "go", "port": 8080 }
```

A Node.js app with a required setting, carrying Node.js:

```json
{
  "name": "worker",
  "type": "node",
  "runtime": "node 22",
  "include_runtime": true,
  "start": "node src/server.js",
  "env": { "QUEUE_URL": "required" }
}
```

A Next.js app published to the hub:

```json
{ "name": "shop", "type": "nextjs", "repo": "acme/shop", "runtime": "node 22" }
```

A static site built with Vite:

```json
{ "name": "docs", "type": "static", "build": "npm run build", "static": "dist" }
```

A Flask app:

```json
{ "name": "blog", "type": "python", "start": "gunicorn app:app -b 0.0.0.0:$PORT" }
```
