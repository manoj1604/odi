# TODO

odi's first releases keep to a small core, so it's quick to pick up:
`build`, `run`, `inspect`, `cd` into a file, `bundle`, and publishing on
the hub (`login`, `push`, `pull`, `versions`). Everything below is either
already in the code as an **experimental** command (it works; `odi help
--all` lists it, and it may change before it joins the core) or planned.

## Experimental now, for a later release

- **Parts and sets** (`odi set`, `odi runtime`, `odi part`): runtimes as
  shared parts, one copy for every app that pins it; themes and mods
  layered over an app; services beside it. [docs/sets.md](docs/sets.md)
- **Fast starts** (`odi profile`, `odi fill`): record what an app reads
  while starting and fetch it in one request; fill a machine ahead of the
  first run. [docs/registry.md](docs/registry.md)
- **Tools for AI agents** (`odi exec`, `odi mcp`, `odi attach`): a packaged
  tool as a command or a stdio MCP server; parts added to a running set.
  [docs/agents.md](docs/agents.md)
- **Browser apps and games** (`odi web`): WebAssembly and WebGPU apps run
  from the file in a browser tab, offline too. [docs/browser.md](docs/browser.md)
- **Signing and trust** (`odi key`, `odi sign`, `--trust`).
  [docs/signing.md](docs/signing.md)
- **More on the hub**: `odi tag` (roll back, promote), `odi diff` (what
  changed, and what an upgrade fetches), `odi search`, `odi repos`,
  `odi rm`, `odi usage`.
- **Starting an app by hand with its own setup** (`odi shell`): its
  runtime on PATH, its environment, its static files served beside it.
- **odi serving a Next.js app's static files** (`odi build --static odi`):
  cached forever, CDN-ready.
- **Odds and ends**: `odi pack` (a folder already laid out), `odi verify`,
  `odi extract`, `odi completions`.

## Planned

- `include_runtime` for Python (there's no official standalone Python
  build to download; python-build-standalone is the likely source).
- Better compression: a shared dictionary and grouping small files
  (measured on text: 74% smaller today, 79–86% with these).
- A lazy disk for microVMs: boot without unpacking the whole file.
- `cd` into a file on macOS without unpacking it.
- Windows.
- Packages as ODI files, in place of `node_modules`.
