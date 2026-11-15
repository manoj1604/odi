# Parts and sets

A **set** is a small ODI file whose manifest lists other ODI files, its
**parts**, by digest. When a set runs, its parts are put together into one
tree, as if they were one file. Because each part is pinned by digest, a
set always means exactly the same files, and a part used by many sets is
stored and fetched once.

```sh
odi set web.odi --runtime acme/node:22 --overlay theme.odi --part prices=prices.odi
odi run web.set.odi
```

`PART` is either an ODI file or a name in a registry (`acme/node:22`). A
part that's a file is also added to this machine's store of parts.

## Roles

| Flag | Role | Where its files go |
|---|---|---|
| `MAIN` (the first argument) | `main` | merged into the tree |
| `--runtime PART` | `runtime` | merged into the tree (usually `runtime/`) |
| `--overlay PART` | `overlay` | merged over what's there: a later file replaces an earlier one |
| `--part NAME=PART` | none | under `parts/NAME/` |

Parts are merged in order, and the set's own files, if it has any, go on
top. `odi inspect` lists each part and every file one part replaced in
another, so a mod or theme is easy to review.

Sets can contain sets, up to 8 deep.

## Runtimes as parts

Instead of putting Node.js inside every app, make it a part once and pin
it:

```sh
odi runtime node 22.23.3 --platform linux/arm64   # from nodejs.org, checked against its SHA-256
odi build --runtime node-22.23.3-linux-arm64.odi   # the app pins it by digest, holds none of it
```

`odi runtime node` downloads nodejs.org's release and checks it against the
published checksums, so the same release makes the same part, with the same
digest, on any machine. Any other runtime can be packed from a folder with
a `bin/`: `odi runtime python 3.13.1 ./python`.

When the app runs, odi finds the pinned runtime in this machine's store
(`odi part add`, `odi part ls`) or in a registry, and puts its `bin/` first
on `PATH`. Unpacked runtime files are shared between apps through hard
links, so ten apps on one runtime use the disk of one.

## Services: plugs

A part can say what it provides and needs in its manifest:

```json
{
  "name": "prices",
  "processes": [{ "name": "web", "command": ["node", "server.js"], "port": { "env": "PORT", "protocol": "http" } }],
  "provides": [{ "plug": "http", "name": "prices" }]
}
```

| Plug | What it is |
|---|---|
| `http`, `tcp` | a service: when the set runs, the part's process starts on a port of its own |
| `cli` | command-line tools: the part's `app/bin` is put on `PATH` |
| `mcp` | an MCP server for AI agents, run with `odi mcp` ([agents](agents.md)) |

Every process in the set gets `ODI_PLUG_<NAME>_URL` for each `http` or
`tcp` service (`ODI_PLUG_PRICES_URL=http://127.0.0.1:41873`). If the set or
any of its parts lists a plug under `needs` that no part provides, `odi run`
refuses to start and names it.

Parts can also be added to a set while it runs: see
[`odi attach`](agents.md#adding-a-part-to-a-running-set).

## Format

A set's manifest has `"odi": "0.2"` and a `parts` list. The format is
described in the [specification](../SPEC.md#59-parts-and-sets-02-draft).
