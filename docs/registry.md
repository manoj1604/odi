# Registries

A registry keeps ODI files under names like `acme/web:1.4` and serves them
in pieces, so publishing sends only what's new and running reads only
what's needed.

## Names

`NAMESPACE/NAME[:TAG]` or `NAMESPACE/NAME@sha256:DIGEST`. A namespace is an
account or organization. A tag (`1.4`, `latest`, `main`) can move; a digest
never changes. With neither, the tag is `latest`.

## Commands

```sh
odi login                               # asks for a token (or --token)
odi push acme/web:1.4                   # the .odi file in this folder, or name the files
odi push acme/web:1.4 web-amd64.odi web-arm64.odi   # one tag, one build per platform
odi pull acme/web:1.4                   # the build for this machine, checked
odi tags acme/web
odi rm acme/web:old                     # a tag; NAME@sha256:… a build; NAME --repo everything
odi usage                               # what your organization stores, and its limits
odi logout
```

Pushing the same build twice stores nothing new, and a push sends only the
pieces the registry doesn't have: after a one-line change to a 14 MB
Next.js app, about 128 KB. A pull checks every piece, the digest it asked
for and every chunk, and writes the same file byte for byte.

`--public` on `push` makes a new repository public; repositories start
private.

## Which registry

The default is `https://hub.odisssi.com`. Another one can be named, in
order of precedence: in the name itself (`registry.example.com/acme/web:1`),
by `ODI_REGISTRY`, or by `odi login --registry URL`. `ODI_TOKEN` gives a
token without logging in (for CI).

## Running from a registry

```sh
odi run acme/web:1.4
```

On Linux the file is mounted with FUSE and read as the program asks for
it; on macOS it's fetched whole first. Pieces fetched are kept in a cache
on this machine, shared by every file that has them.

### Fast starts

Programs read the same files every time they start. Record which ones,
once, and a runner fetches all of them in one request before the program
asks:

```sh
odi profile app.odi --path / --path /pricing     # Linux; or odi build --profile
odi push acme/web:1.4 app.odi
```

For hosts that know what they'll run, `odi fill acme/web:1.4` fetches the
start list ahead of time (`--all` fetches everything), so the first request
reads nothing from the network.

Measured with a mock registry 50 ms away (Linux VM): Express, Flask, Go
and Next.js apps answered their first request in 0.3–0.6 s with a start
list; the Next.js app took 23 s without one.

## The protocol

Registries speak a small HTTP and JSON protocol, open for anyone to
implement. Files are stored as **pieces**, exact byte ranges of the file
named by their SHA-256: each chunk, and the bytes between chunks. A
registry never decompresses or rewrites anything, so it can run anywhere,
including edge platforms without zstd. Clients check every piece, the
digest and every chunk.

Requests use bearer tokens; public repositories need none to read. `REPO`
is `NAMESPACE/NAME` and digests in paths are hex.

| Request | Does |
|---|---|
| `GET /v1/repos/REPO` | visibility, tags, builds, the newest build's manifest and README |
| `PUT /v1/repos/REPO` `{public}` | change visibility |
| `GET /v1/repos/REPO/tags`, `GET …/tags/TAG` | tags and their digests (`?builds=1`: each build's size and pieces too, optional) |
| `PUT /v1/repos/REPO/tags/TAG` `{platforms: [{platform, digest}], add?}` | point a tag at builds, one per platform |
| `POST /v1/repos/REPO/digests` `{digest, size, segments: [{hash, length}], manifest, readme}` | start a push: answers `{ready, missing}` |
| `PUT /v1/repos/REPO/segments/HASH` | upload a piece (its hash is checked) |
| `POST /v1/repos/REPO/digests/DIGEST/commit` | finish a push (`409` with what's still missing) |
| `GET /v1/repos/REPO/digests/DIGEST` | size, manifest and pieces (`?parts=1`: a set's parts' too, optional) |
| `GET /v1/repos/REPO/segments/HASH` | one piece |
| `POST /v1/repos/REPO/segments/batch` `{hashes}` | many pieces in one answer, back to back (optional: clients fall back to single pieces) |
| `DELETE /v1/repos/REPO/tags/TAG`, `…/digests/DIGEST`, `/v1/repos/REPO` | delete a tag, a build, a repository |
| `GET /v1/usage` | what the organization stores, and its limits |

Optional requests may be missing: clients fall back when they get 404, 405
or 501. [`bench/mockhub.py`](../bench/mockhub.py) is a small working
registry, used by the benchmarks.
