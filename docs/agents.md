# Tools for AI agents

An ODI file can hold a command-line tool or an MCP server just as well as
a web app: one file, pinned by digest, that runs the same everywhere. odi
gives agents three ways in.

## Run a tool in the terminal: `odi exec`

```sh
odi exec acme/pdf-tools:2 -- merge a.pdf b.pdf -o out.pdf
```

Runs one of the file's processes with the arguments after `--`, in this
terminal, and exits with its exit code. `--process NAME` picks a process
when there are several; `--part NAME` picks one of a set's parts.

## Serve an MCP server: `odi mcp`

A part that provides an `mcp` plug is a stdio MCP server:

```json
{
  "name": "pdf",
  "processes": [{ "name": "mcp", "command": ["node", "server.js"] }],
  "provides": [{ "plug": "mcp", "name": "pdf", "process": "mcp" }]
}
```

`odi mcp` runs it on stdin and stdout, which is what an agent's MCP
configuration expects:

```json
{
  "mcpServers": {
    "pdf": { "command": "odi", "args": ["mcp", "acme/pdf:1"] }
  }
}
```

With several `mcp` plugs in one file or set, `--name PLUG` picks one.
`-e KEY=VALUE` passes settings, and `--trust KEY` runs only what that key
signed.

## Start tools fast: `odi profile --mcp`

Tools built on big SDKs read hundreds of files while starting. Record which
ones, once:

```sh
odi profile tool.odi --mcp          # Linux: asks it initialize and tools/list
odi push acme/tool:1 tool.odi
```

odi runs the tool mounted, sends it `initialize` and `tools/list` (or the
JSON-RPC lines of `--input FILE`), and keeps what it read, in order, as the
file's start list. Run from a registry, the start list then arrives in one
request. In our test, a tool on the official MCP SDK took 220 requests
without a start list and 2 with one, and started as fast as from a local
file.

## Adding a part to a running set

On Linux and macOS, `odi run` listens for parts to add while it runs. The
first argument of `odi attach` is the running set's name:

```sh
odi run board.set.odi              # a set named "board" (odi inspect shows the name)
odi attach board prices.odi --as prices
odi attach board acme/search:3
```

An attached part is checked against your trusted keys (if you have any),
read lazily if it comes from a registry, and its `http` and `tcp` services
start. If it stops, the set keeps running. Two parts can't share a name.

A running process's environment can't change, so live plugs are listed in a
file instead: every process gets `ODI_PLUGS`, the path of a JSON file that
is rewritten whenever a part joins:

```json
{
  "plugs": [
    { "plug": "http", "name": "prices", "part": "prices", "url": "http://127.0.0.1:41873" },
    { "plug": "mcp", "name": "search", "part": "search", "root": "/tmp/odi-…/search" }
  ]
}
```

Services known when the set starts are also given as
`ODI_PLUG_<NAME>_URL` variables ([parts and sets](sets.md#services-plugs)).
