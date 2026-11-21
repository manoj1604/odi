# Getting started

Build a project into one ODI file, run it, look inside it, and publish it.
For every flag, see the [command reference](cli.md) or `odi help COMMAND`.

## Install

```sh
curl -fsSL https://github.com/manoj1604/odi/releases/latest/download/install.sh | sh
odi version
```

Or build from source: `cargo install --git https://github.com/manoj1604/odi odi`.

## Build

Every project has an [`odi.json`](odi-json.md) that says what it is.
`odi init` writes one from what it finds:

```sh
cd shop
odi init
```

```json
{
  "$schema": "https://raw.githubusercontent.com/manoj1604/odi/main/schema/odi.schema.json",
  "name": "shop",
  "type": "nextjs",
  "runtime": "node 22"
}
```

Then build:

```sh
odi build
```

odi runs the project's own build for its `type`:

| `type` | What odi does |
|---|---|
| `go` | `go build` with `CGO_ENABLED=0`: one static executable |
| `node` | `npm ci`, `npm run build` if there's a build script, production `node_modules` |
| `nextjs` | as `node`, with `output: "standalone"` in `next.config` |
| `python` | `pip install` into the app (Flask, FastAPI, Django) |
| `static` | the files to serve, built first if the project says how |

It writes `NAME.odi` (or `-o FILE`) and prints its digest. Building the
same commit again gives the same digest.

Add what odi can't work out to `odi.json`: the start command, the port,
environment variables, and `include_runtime` to put Node.js itself in the
file, so the app runs on a machine without it:

```json
{
  "name": "shop",
  "type": "nextjs",
  "runtime": "node 22",
  "include_runtime": true,
  "env": { "NODE_ENV": "production", "DATABASE_URL": "required" }
}
```

Every key is in [the odi.json specification](odi-json.md). To build for
another machine: `odi build --platform linux/amd64`.

## Run

```sh
odi run shop.odi
odi run shop.odi --port 3000 -e DATABASE_URL=postgres://localhost/shop
```

The app gets its port in `PORT`, the environment variables you give with
`-e` (odi asks for any it requires), and a scratch folder in `TMPDIR`.
Ctrl-C stops it and cleans up.

## Look inside

```sh
odi inspect shop.odi      # its processes, runtime, size and digest
```

Or `cd` into it. Add one line to your shell's startup file, once:

```sh
# ~/.zshrc
command -v odi >/dev/null && eval "$(odi hook zsh)"
# ~/.bashrc
command -v odi >/dev/null && eval "$(odi hook bash)"
# ~/.config/fish/config.fish
command -q odi; and odi hook fish | source
```

(The `command -v odi` part means a new terminal still starts fine if you
remove odi one day.) Open a new terminal, or `source ~/.zshrc`.

Then an ODI file is a folder you can `cd` into:

```sh
cd shop.odi                # app/, meta/ and the rest
ls app
cat app/package.json
cd shop.odi/app            # straight to a folder inside it
cd acme/shop:1.0           # a name on the hub too (with a tag)
odi close                  # let them go when you're done
```

It's read-only. `cd ..` at its top comes back out to the folder the file is
in, and tab after `cd` offers `.odi` files. On Linux the file is mounted and
read as you look; on macOS it's unpacked once (into `~/.odi/open`), and
`cd` into it again is instant. Any other `cd` is your shell's own, exactly
as before. Inside, you can start the app yourself too:
`cd shop.odi/app && node server.js`.

## Publish

```sh
odi login                       # paste an API key: dev.odisssi.com/dashboard#keys
odi push acme/shop:1.0.0        # only what the hub doesn't have is sent
odi push acme/shop --bump patch # later: 1.0.1, and latest moves to it
odi versions acme/shop          # what's published
odi run acme/shop:1.0           # the newest 1.0.x, from anywhere
odi pull acme/shop:1.0          # or download the file, checked
```

On Linux, `odi run acme/shop:1.0` starts the app before the whole file has
downloaded: it's read as the app asks for it. More in
[publishing on the hub](hub.md): versions, ranges and CI.

## Run it where odi isn't installed

`odi bundle` makes one executable that is the app: a small odi with the ODI
file inside, unchanged. Copy it to a VM, a microVM or a server and run it;
nothing is installed there:

```sh
odi bundle shop.odi -o shop     # for Linux on this CPU; --platform linux/amd64 for another
scp shop vm: && ssh vm ./shop   # serves the app, as odi run does
./shop --odi-file shop.odi      # the ODI file back out, the same digest
```

The bundle runs anything the file carries. For a Node.js app on a machine
with no Node.js, build it with `"include_runtime": true` in `odi.json`. In
a microVM, the bundle can be the only thing you add to the disk: start it
from the guest's init (`exec /shop --port 8080`).

## Next

- [Publishing on the hub](hub.md): versions, `--bump`, ranges, CI.
- [Command reference](cli.md): every command and flag.
- [TODO.md](../TODO.md): what's coming, and the experimental commands
  (`odi help --all`) you can try today.
