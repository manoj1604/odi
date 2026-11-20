# Configuration

odi has no configuration file to write by hand. It keeps a few things in
two folders, and reads a few environment variables.

## Files

| Where | What |
|---|---|
| `~/.odi/config.json` | the registry sign-in (`odi login`), mode 0600 |
| `~/.odi/keys/NAME.key` | signing keys (`odi key new`), mode 0600 |
| `~/.odi/trust` | public keys to trust, one per line ([signing](signing.md)) |
| cache folder: `~/Library/Caches/odi` (macOS), `$XDG_CACHE_HOME/odi` or `~/.cache/odi` (Linux) | everything odi can fetch again |

Inside the cache folder:

| Folder | What |
|---|---|
| `parts/` | this machine's store of parts (`odi part ls`) |
| `pieces/` | registry pieces, by SHA-256, shared by every file |
| `builds/` | registry listings of builds |
| `files/` | unpacked files shared between apps (hard links, read-only) |
| `run/` | sockets of running sets (`odi attach`) |

Deleting the cache folder is safe: odi fetches or unpacks again what it
needs.

## A project's odi.json

`odi build` merges a project's `odi.json` into the manifest, so it can set
any manifest field (`name`, `version`, `env`, `processes`…). One more key
isn't part of the manifest: `"repo": "acme/web"`, the repository `odi push`
and `odi versions` use when no name is given ([publishing](hub.md)).

## Environment variables

| Variable | What it does |
|---|---|
| `ODI_HOME` | use this folder instead of `~/.odi` |
| `ODI_CACHE` | use this cache folder |
| `ODI_REGISTRY` | the registry's address (default `https://hub.odisssi.com`) |
| `ODI_TOKEN` | a registry token, instead of `odi login` (for CI) |
| `ODI_TRUST` | trusted public keys, separated by commas |
| `ODI_PIECE_CACHE=0` | don't keep registry pieces on this machine |
| `ODI_NODE_MIRROR` | where `odi runtime node` downloads from (default `https://nodejs.org/dist`) |
| `ODI_VERBOSE` | `odi exec` reports what it fetched |
| `ODI_TRACE=FILE` | append every registry read to FILE (for debugging) |
| `SOURCE_DATE_EPOCH` | the build time `odi build` records (default: the last commit's) |

## What a program gets

When odi runs a program, it adds these to the environment:

| Variable | Value |
|---|---|
| `PORT` (or the manifest's port variable) | the port to listen on |
| `ODI_ROOT` | where the file's contents are |
| `ODI_DIGEST` | the file's digest |
| `TMPDIR` | a scratch folder, removed when it stops |
| `PATH` | the runtime's `bin/` and `cli` parts' `bin/` first |
| `ODI_PLUG_<NAME>_URL` | where each service of the set listens |
| `ODI_PLUGS` | a JSON file listing every plug, rewritten as parts are attached |

Plus the manifest's `env.defaults` and anything given with `-e KEY=VALUE`.
