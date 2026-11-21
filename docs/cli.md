# Command reference

Made from `odi help` by `docs/gen-cli.sh`: don't edit by hand.

```
odi: one file for a program's build output, any language, runs anywhere

Usage: odi COMMAND [ARGUMENTS] [FLAGS]

Build and run:
  init         Write the project's odi.json, from what odi finds in it
  build        Build a project into an ODI file (Go, Node.js, Next.js, Python, static)
  run          Run an ODI file, or one from a registry, on this machine
  bundle       Make one executable that runs the app, with nothing to install where it runs

Look inside:
  inspect      Show what's in it: processes, static files, runtime, parts, signatures, size
  hook         Set up cd into .odi files: cd app.odi (add its line to your shell)
  close        Let go of the files cd went into

Publish on the hub:
  login        Sign in to the hub, in your browser
  push         Publish a version (one build per platform), and move latest to it
  pull         Download a file (this machine's platform), checked against its digest
  versions     List a repository's versions and tags: platforms, size, when
  logout       Forget the registry sign-in

Help:
  help         Show help for odi, or for one command
  version      Show odi's version

Run 'odi help COMMAND' (or odi COMMAND --help) for its flags and examples.
More commands, experimental: odi help --all
Docs: https://github.com/manoj1604/odi/tree/main/docs
```

## odi init

```
odi init: Write the project's odi.json, from what odi finds in it

Usage:
  odi init [PROJECT] [flags]

Every project odi builds has an odi.json: its name, its type (go, node,
nextjs, python, static) and anything odi can't work out (the runtime, whether
to include it, build and start commands, the port, environment variables). odi
init writes one with what it finds; edit it as you like. Specification:
docs/odi-json.md.

Flags:
  --type NAME  what kind of project, when odi can't tell
  --name NAME  the app's name (default: package.json's, go.mod's, or the folder's)
  -h, --help   this help

Examples:
  odi init
  odi init --type static
  odi init ../shop --name shop
```

## odi build

```
odi build: Build a project into an ODI file (Go, Node.js, Next.js, Python, static)

Usage:
  odi build [PROJECT] [flags]

Runs the project's own build (go build, npm ci and npm run build, pip) and
packs what it produced, with a manifest that says how to start it. The same
commit builds to the same digest.

It needs the project's odi.json (odi init writes one): its name and type, and
anything odi can't work out:
  {"name": "shop", "type": "nextjs", "runtime": "node 22", "include_runtime": true,
   "env": {"DATABASE_URL": "required"}}
include_runtime puts Node.js itself in the file (the official release for the
target, checked), so it runs where Node.js isn't installed. Every key:
docs/odi-json.md.

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
  --static app|odi    Next.js: who serves .next/static and public/: app (default) or odi (cached forever, CDN-ready)
  -h, --help          this help

Examples:
  odi build
  odi build ./api -o api.odi
  odi build --platform linux/amd64
  odi build --runtime node-22.23.3-linux-arm64.odi
  odi build --profile --path / --path /pricing
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

## odi bundle

```
odi bundle: Make one executable that runs the app, with nothing to install where it runs

Usage:
  odi bundle FILE [-o OUT] [--platform OS/ARCH] [--runner FILE]

Writes a small static odi for the target machine with the ODI file after it,
unchanged (the same digest). For a machine with no Node.js, build the app with
its runtime inside ("include_runtime": true in odi.json). Copy it to a VM, a
microVM or a server and run it: it serves the app as odi run would (--port, -e
KEY=VALUE). OUT --odi-file app.odi gets the ODI file back out. The runner is
this odi when it fits the target, else odi's release for it, checked against
its SHA256SUMS.

Flags:
  -o OUT              the executable to write (default: the file's name, without .odi)
  --platform OS/ARCH  the machine it's for, when the file runs anywhere (default: linux and this CPU)
  --runner FILE       the odi to bundle with (default: this one if it fits, else the release for the target)
  -h, --help          this help

Examples:
  odi bundle app.odi -o app && scp app vm: && ssh vm ./app
  odi bundle app.odi --platform linux/amd64
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

## odi hook

```
odi hook: Set up cd into .odi files: cd app.odi (add its line to your shell)

Usage:
  odi hook zsh|bash|fish

Add one line to your shell's startup file (it does nothing if odi is removed
later):
  zsh  (~/.zshrc):   command -v odi >/dev/null && eval "$(odi hook zsh)"
  bash (~/.bashrc):  command -v odi >/dev/null && eval "$(odi hook bash)"
  fish (config.fish): command -q odi; and odi hook fish | source
Then cd app.odi, cd app.odi/app or cd acme/web:1.4 (a hub name with a tag)
goes inside, read-only, and ls, cat and the rest work as usual; cd .. at its
top comes back out to the folder the file is in. Tab after cd offers .odi
files. On Linux the file is mounted and read as you look (FUSE); elsewhere
it's unpacked once under ~/.odi/open. odi close lets them go. Any other cd is
the shell's own, unchanged.

Examples:
  odi hook zsh
```

## odi close

```
odi close: Let go of the files cd went into

Usage:
  odi close [FILE|NAME]

Examples:
  odi close
  odi close app.odi
```

## odi login

```
odi login: Sign in to the hub, in your browser

Usage:
  odi login [flags]

Opens the website in your browser: sign in there (or sign up if you're new)
and allow this computer; odi gets a token of its own. With no browser on this
machine (a server, CI): --token, or ODI_TOKEN, with an API key from your
dashboard. The hub is hub.odisssi.com; --registry (or $ODI_REGISTRY) names
another, which takes a token.

Flags:
  --token TOKEN   an account token, or an organization's API key
  --registry URL  another registry's address
  -h, --help      this help

Examples:
  odi login
  odi login --token $ODI_KEY
  odi login --registry https://registry.example.com --token $TOKEN
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

## odi logout

```
odi logout: Forget the registry sign-in

Usage:
  odi logout

Examples:
  odi logout
```

## odi help

```
odi help: Show help for odi, or for one command

Usage:
  odi help [COMMAND]

Examples:
  odi help run
```

## odi version

```
odi version: Show odi's version

Usage:
  odi version

Examples:
  odi version
```

# Experimental commands

These work, and may change before they join the list above (see
[TODO.md](../TODO.md)). `odi help --all` lists them.

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

## odi shell

```
odi shell: Open a shell inside it, ready to start it by hand (cd into the file)

Usage:
  odi shell FILE|NAME [PATH] [flags] [-- COMMAND...]

You land in the app's folder (app/) with everything odi run would give it: the
file's runtime first on PATH (node, python), its environment, PORT, and its
writable folders. odi prints the start command; when the app has static files
(Next.js's /_next/static), odi serves them on --port (8080) and passes the
rest to the app, as odi run does. On Linux the file is mounted read-only and
read as you go (FUSE); elsewhere you get a copy of your own, removed when you
exit. ODI_ROOT is the file's top.

Flags:
  --port PORT         where odi serves the app's static files (default: the app's port, else 8080)
  -e KEY=VALUE        KEY=VALUE for the app (repeat)
  --use-path-runtime  use this machine's runtime for one pinned by digest, unchecked
  -h, --help          this help

Examples:
  odi shell app.odi            # then: node server.js, and open http://localhost:8080
  odi shell acme/web:1.4 -e DATABASE_URL=postgres://localhost/web
  odi shell app.odi static     # look around the static files
  odi shell app.odi -- node -e 'console.log(process.version)'
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
