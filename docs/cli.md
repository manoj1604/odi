# Command reference

Made from `odi help` by `docs/gen-cli.sh`: don't edit by hand.

```
odi: one file for a program's build output, any language, runs anywhere

Usage: odi COMMAND [ARGUMENTS] [FLAGS]

Build:
  build        Build a project into an ODI file (Go, Node.js, Next.js, Python, static)
  pack         Pack a folder already laid out (app/, static/, runtime/, meta/)

Run:
  run          Run an ODI file, or one from a registry, on this machine
  exec         Run one of its processes in this terminal (a command-line tool)
  mcp          Serve what it provides as an mcp plug, as a stdio MCP server (for AI agents)
  attach       Add a part to a set that's running
  web          Run a browser app (its static/ files) in a browser tab

Parts and sets:
  set          Make a set: parts pinned by digest, put together when it runs
  runtime      Make a runtime part: one copy serves every app that pins it
  part         This machine's store of parts

Fast starts:
  profile      Record what it reads while starting: its start list (Linux)
  fill         Fetch its start list (or all of it) onto this machine before its first run

Inspect and trust:
  inspect      Show what's in it: processes, static files, runtime, parts, signatures, size
  verify       Check every byte against its digest, and its signatures
  diff         Compare two builds: what changed, and what a machine with the old one must fetch
  extract      Unpack it into a folder (a set: put together first)
  key          Make or list signing keys (ed25519, in ~/.odi/keys)
  sign         Sign it with one of your keys (its digest doesn't change)

Registry:
  login        Sign in to a registry
  logout       Forget the registry sign-in
  push         Publish a version (one build per platform), and move latest to it
  pull         Download a file (this machine's platform), checked against its digest
  versions     List a repository's versions and tags: platforms, size, when
  tag          Point tags at a version's builds, without pushing again (promote, roll back)
  search       Find public repositories (by name or language)
  repos        List your organization's repositories
  rm           Delete a tag, a build, or a repository with its builds
  usage        Show what your organization stores, and its limits

This command:
  help         Show help for odi, or for one command
  completions  Print a shell completion script
  version      Show odi's version

Run 'odi help COMMAND' (or odi COMMAND --help) for its flags and examples.
Docs: https://github.com/manoj1604/odi/tree/main/docs
```

## odi build

```
odi build: Build a project into an ODI file (Go, Node.js, Next.js, Python, static)

Usage:
  odi build [PROJECT] [flags]

Runs the project's own build (go build, npm ci and npm run build, pip) and
packs what it produced, with a manifest that says how to start it. A
Procfile's web: line and an odi.json (any manifest fields) in the project
override what odi finds. The same commit builds to the same digest.

--platform builds for another machine: Go, Node.js and Next.js swap compiled
packages for the target's, at the lockfile's versions; Python builds for this
machine.

Flags:
  -o FILE             the ODI file (default: NAME.odi)
  --platform OS/ARCH  OS/ARCH to build for (Go, Node.js, Next.js; Python: this machine's)
  --no-build          don't run the project's build script
  --profile           then run it and keep what it reads while starting (Linux; see odi profile)
  --path PATH         with --profile: a URL path to ask for (repeat; default /)
  -e KEY=VALUE        with --profile: KEY=VALUE for the program (repeat)
  --port PORT         with --profile: the port to run it on
  --use-path-runtime  with --profile: run a runtime pinned by digest with this machine's
  --runtime PART      a runtime part (file or registry name) to pin, by digest and exact version
  --compress LEVEL    default, or max (zstd 19: about 10% smaller, as fast to read)
  -h, --help          this help

Examples:
  odi build
  odi build ./api -o api.odi
  odi build --platform linux/amd64
  odi build --runtime node-22.23.3-linux-arm64.odi
  odi build --profile --path / --path /pricing
```

## odi pack

```
odi pack: Pack a folder already laid out (app/, static/, runtime/, meta/)

Usage:
  odi pack DIR [flags]

For build output odi doesn't build itself. The manifest comes from
DIR/odi.manifest.json (or --manifest) and isn't packed as a file.

Flags:
  --compress LEVEL  default, or max (zstd 19: about 10% smaller, as fast to read)
  -o FILE           the ODI file to write (default: NAME.odi)
  --manifest FILE   the manifest (default: DIR/odi.manifest.json, kept out of the file)
  -h, --help        this help

Examples:
  odi pack out/ -o site.odi
  odi pack dist --manifest odi.manifest.json --compress max
```

## odi run

```
odi run: Run an ODI file, or one from a registry, on this machine

Usage:
  odi run FILE|NAME [flags]

Static files are served by odi; requests for anything else go to the web
process. A set's parts are put together first, and parts that provide http or
tcp plugs run as services beside it.

From a registry, on Linux, the file is mounted (FUSE, fusermount3) and read as
the program asks for it, starting with its start list; elsewhere it's fetched
whole first. --trust (or ~/.odi/trust, $ODI_TRUST) refuses anything not signed
by those keys.

Flags:
  --trust KEY         run it only if this key signed it (repeat; also ~/.odi/trust, $ODI_TRUST)
  --port PORT         the port to serve on (default: web's default, else 8080)
  -e KEY=VALUE        KEY=VALUE for the program (repeat)
  --use-path-runtime  run a runtime pinned by digest with this machine's, unchecked
  -h, --help          this help

Examples:
  odi run app.odi
  odi run acme/web:1.4 --port 3000 -e DATABASE_URL=postgres://...
  odi run acme/web:1.4 --trust ed25519:3f1c...
```

## odi exec

```
odi exec: Run one of its processes in this terminal (a command-line tool)

Usage:
  odi exec FILE|NAME [flags] [-- ARGS...]

Arguments after -- go to the program. Its exit code is odi's.

Flags:
  --process NAME      which process (default: the only one, or the first)
  --part NAME         a part of the set's, not the set's own
  -e KEY=VALUE        KEY=VALUE for the program (repeat)
  --use-path-runtime  run a runtime pinned by digest with this machine's, unchecked
  --trust KEY         run it only if this key signed it (repeat; also ~/.odi/trust, $ODI_TRUST)
  -h, --help          this help

Examples:
  odi exec acme/pdf-tools:2 -- merge a.pdf b.pdf -o out.pdf
  odi exec set.odi --part lint -- --fix .
```

## odi mcp

```
odi mcp: Serve what it provides as an mcp plug, as a stdio MCP server (for AI agents)

Usage:
  odi mcp FILE|NAME [flags]

The command an agent's MCP configuration names: {"command": "odi", "args":
["mcp", "acme/pdf:1"]}.

Flags:
  --name NAME         which mcp plug, when there are several
  -e KEY=VALUE        KEY=VALUE for the program (repeat)
  --use-path-runtime  run a runtime pinned by digest with this machine's, unchecked
  --trust KEY         run it only if this key signed it (repeat; also ~/.odi/trust, $ODI_TRUST)
  -h, --help          this help

Examples:
  odi mcp acme/pdf:1
  odi mcp tools.set.odi --name search
```

## odi attach

```
odi attach: Add a part to a set that's running

Usage:
  odi attach RUNNING PART [flags]

RUNNING is the running set's name (its manifest name, as odi inspect shows
it). Linux and macOS. The part's services start, and its plugs are added to
the file $ODI_PLUGS names, which every process of the set can read.

Flags:
  --as NAME   the part's name in the set (default: its own)
  -h, --help  this help

Examples:
  odi attach web prices.odi --as prices
  odi attach web acme/search:3
```

## odi web

```
odi web: Run a browser app (its static/ files) in a browser tab

Usage:
  odi web FILE... [flags]

Serves the browser runner (built into odi) and the files on localhost. The tab
reads the ODI file with Range requests, fetching only what the app asks for,
and checks every byte; it can keep the app for offline play. Any static host
with Range requests can serve the runner (web/) and ODI files the same way.

Flags:
  --port PORT  the port (default 8080)
  -h, --help   this help

Examples:
  odi web game.odi
  odi web game.odi mod.set.odi --port 9000
```

## odi set

```
odi set: Make a set: parts pinned by digest, put together when it runs

Usage:
  odi set MAIN [flags]

A set is a small ODI file whose manifest lists other ODI files (parts) by
digest. main, runtime and overlay parts are merged into one tree, a later file
replacing an earlier one; a --part NAME=PART is under parts/NAME/. PART is an
ODI file or a name in a registry.

Flags:
  -o FILE           the set to write (default: NAME.set.odi)
  --name NAME       the set's name (default: the main part's)
  --runtime PART    the runtime part (default: the one main pins, if it's in the store)
  --overlay PART    a part merged over main (repeat; later ones win)
  --part NAME=PART  a part under parts/NAME/ (repeat)
  -h, --help        this help

Examples:
  odi set web.odi --runtime acme/node:22
  odi set game.odi --overlay hd-textures.odi -o game-hd.set.odi
  odi set web.odi --part prices=prices.odi --part docs=acme/docs:1
```

## odi runtime

```
odi runtime: Make a runtime part: one copy serves every app that pins it

Usage:
  odi runtime node [VERSION] [flags]
  odi runtime NAME VERSION DIR [flags]

odi runtime node downloads nodejs.org's release for the platform, checks it
against the published SHA-256, and packs it: the same release gives the same
digest on any machine. NAME VERSION DIR packs any folder with bin/.

Flags:
  -o FILE             the ODI file (default: NAME-VERSION-OS-ARCH.odi)
  --platform OS/ARCH  OS/ARCH it's for (default: this machine)
  -h, --help          this help

Examples:
  odi runtime node 22.23.3 --platform linux/arm64
  odi runtime python 3.13.1 ./python-3.13.1
```

## odi part

```
odi part: This machine's store of parts

Usage:
  odi part add FILE...
  odi part ls

Sets find their parts here (by digest) before asking a registry.

Examples:
  odi part add node-22.23.3-linux-arm64.odi
  odi part ls
```

## odi profile

```
odi profile: Record what it reads while starting: its start list (Linux)

Usage:
  odi profile FILE [flags]
  odi profile FILE --mcp [--input FILE] [-o FILE]

Runs it mounted, asks for each --path once it's up (or, with --mcp, sends a
stdio tool initialize and tools/list), and keeps what it read, in order, in
the file. Run from a registry, all of that then comes in one request.

Flags:
  --mcp               it's a stdio MCP server: ask it initialize and tools/list (or --input), not URL paths
  --input FILE        with --mcp: a file of JSON-RPC lines to send it instead
  -o FILE             the ODI file to write (default: FILE, replaced)
  --path PATH         a URL path to ask for once it's up (repeat; default /)
  --port PORT         the port to serve on (default: web's default, else 8080)
  -e KEY=VALUE        KEY=VALUE for the program (repeat)
  --use-path-runtime  run a runtime pinned by digest with this machine's, unchecked
  -h, --help          this help

Examples:
  odi profile app.odi --path / --path /docs
  odi profile tool.odi --mcp
```

## odi fill

```
odi fill: Fetch its start list (or all of it) onto this machine before its first run

Usage:
  odi fill NAME|FILE... [flags]

For hosts that know what they'll run: the first request then reads nothing
from the network.

Flags:
  --all       all of it, not just its start list
  -h, --help  this help

Examples:
  odi fill acme/web:1.4
  odi fill --all acme/web:1.4 acme/api:2
```

## odi inspect

```
odi inspect: Show what's in it: processes, static files, runtime, parts, signatures, size

Usage:
  odi inspect FILE|NAME

Examples:
  odi inspect app.odi
  odi inspect acme/web:1.4
```

## odi verify

```
odi verify: Check every byte against its digest, and its signatures

Usage:
  odi verify FILE [flags]

With --key, one of those keys must have signed it. A set's parts are checked
too.

Flags:
  --key KEY   a public key that must have signed it (repeat: any of them)
  -h, --help  this help

Examples:
  odi verify app.odi
  odi verify app.odi --key ed25519:3f1c...
```

## odi diff

```
odi diff: Compare two builds: what changed, and what a machine with the old one must fetch

Usage:
  odi diff OLD NEW [flags]

OLD and NEW are ODI files or names in a registry (only their indexes are
read).

Flags:
  --all       list every changed file, not just the first 40
  -h, --help  this help

Examples:
  odi diff acme/web:1.4.2 acme/web:1.4.3
  odi diff old.odi new.odi --all
```

## odi extract

```
odi extract: Unpack it into a folder (a set: put together first)

Usage:
  odi extract FILE DIR

Examples:
  odi extract app.odi out/
```

## odi key

```
odi key: Make or list signing keys (ed25519, in ~/.odi/keys)

Usage:
  odi key new [NAME]
  odi key ls

Prints the public key, ed25519:HEX, to give to whoever runs your files.

Examples:
  odi key new
  odi key new release
  odi key ls
```

## odi sign

```
odi sign: Sign it with one of your keys (its digest doesn't change)

Usage:
  odi sign FILE... [flags]

Flags:
  --key NAME  the key to sign with (default: default)
  -h, --help  this help

Examples:
  odi sign app.odi
  odi sign app.odi --key release
```

## odi login

```
odi login: Sign in to a registry

Usage:
  odi login [flags]

Without --token, asks for one. The default registry is hub.odisssi.com;
--registry (or $ODI_REGISTRY) names another.

Flags:
  --token TOKEN   an account token, or an organization's API key
  --registry URL  another registry's address
  -h, --help      this help

Examples:
  odi login
  odi login --registry https://registry.example.com --token $TOKEN
```

## odi logout

```
odi logout: Forget the registry sign-in

Usage:
  odi logout

Examples:
  odi logout
```

## odi push

```
odi push: Publish a version (one build per platform), and move latest to it

Usage:
  odi push [NAMESPACE/NAME[:VERSION]] [FILE...] [flags]

Only what the registry doesn't have yet is sent. The version is the one named
(acme/web:1.4.2), or the next after the newest (--bump patch|minor|major), or
the file's own (odi build takes it from a git tag, package.json or
pyproject.toml). A release also moves latest; a pre-release (2.0.0-rc.1)
doesn't.

A published version doesn't change: pushing other builds to it is refused
(--force moves it). Other tags (latest, staging) move. With several files,
each is the build for its platform; --add adds platforms to a version one CI
job at a time.

Without a name, the repository is odi.json's "repo" ("repo": "acme/web").

Flags:
  --bump PART  publish the next version after the newest one: patch, minor or major
  --tag TAG    another tag to point at it, such as staging (repeat)
  --no-latest  don't move latest to it
  --add        add to the platforms the version or tag already has (one build per platform)
  --public     make the repository public
  --force      move a version that's already published to these builds
  -h, --help   this help

Examples:
  odi push acme/web
  odi push acme/web:1.4.2
  odi push acme/web --bump minor
  odi push acme/web:2.0.0-rc.1 --tag next
  odi push acme/web:1.4.2 web-amd64.odi web-arm64.odi
  odi push acme/web:1.4.2 web-arm64.odi --add
```

## odi pull

```
odi pull: Download a file (this machine's platform), checked against its digest

Usage:
  odi pull NAMESPACE/NAME[:TAG|:RANGE|@sha256:DIGEST] [flags]

Any command that takes a name takes a version range too: acme/web:1.4 is the
newest 1.4.x, acme/web:1 (or ^1.0) the newest 1.x.y, ~1.4.2 the newest 1.4.x
from 1.4.2. A tag with that exact name wins.

Flags:
  -o FILE             where to write it
  --platform OS/ARCH  the platform to get (OS/ARCH)
  -h, --help          this help

Examples:
  odi pull acme/web
  odi pull acme/web:1.4
  odi pull acme/web@sha256:877a... -o web.odi
```

## odi versions

```
odi versions: List a repository's versions and tags: platforms, size, when

Usage:
  odi versions [NAMESPACE/NAME] [flags]

Newest version first, with the other tags pointing at the same builds (latest,
staging). Without a name, odi.json's "repo". (odi tags is the same.)

Flags:
  --all       also list builds no tag points at
  -h, --help  this help

Examples:
  odi versions acme/web
  odi versions acme/web --all
```

## odi tag

```
odi tag: Point tags at a version's builds, without pushing again (promote, roll back)

Usage:
  odi tag NAMESPACE/NAME:FROM TAG... [flags]

FROM is a version, a range, a tag or @sha256:DIGEST. Versions already
published don't move without --force.

Flags:
  --force     move a version that's already published
  -h, --help  this help

Examples:
  odi tag acme/web:1.4.2 latest
  odi tag acme/web:1.3 latest
  odi tag acme/web:staging production
```

## odi search

```
odi search: Find public repositories (by name or language)

Usage:
  odi search [QUERY]

Examples:
  odi search
  odi search nextjs
```

## odi repos

```
odi repos: List your organization's repositories

Usage:
  odi repos

Examples:
  odi repos
```

## odi rm

```
odi rm: Delete a tag, a build, or a repository with its builds

Usage:
  odi rm NAMESPACE/NAME:TAG
  odi rm NAMESPACE/NAME@sha256:DIGEST
  odi rm NAMESPACE/NAME --repo

Flags:
  --repo      delete the repository, with every build
  -h, --help  this help

Examples:
  odi rm acme/web:old
  odi rm acme/scratch --repo
```

## odi usage

```
odi usage: Show what your organization stores, and its limits

Usage:
  odi usage [NAMESPACE]

Examples:
  odi usage
```

## odi help

```
odi help: Show help for odi, or for one command

Usage:
  odi help [COMMAND]

Examples:
  odi help run
```

## odi completions

```
odi completions: Print a shell completion script

Usage:
  odi completions bash|zsh|fish

Add it to your shell's startup file, for example:
  bash: eval "$(odi completions bash)"   (in ~/.bashrc)
  zsh:  eval "$(odi completions zsh)"    (in ~/.zshrc, after compinit)
  fish: odi completions fish > ~/.config/fish/completions/odi.fish

Examples:
  odi completions zsh
```

## odi version

```
odi version: Show odi's version

Usage:
  odi version

Examples:
  odi version
```
