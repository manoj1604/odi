# Publishing on the hub

The hub ([hub.odisssi.com](https://hub.odisssi.com), or any registry that
speaks the [protocol](registry.md#the-protocol)) keeps your builds under
names like `acme/web`, as **versions** (`1.4.2`) and **tags** (`latest`,
`staging`). This page is the everyday workflow: publish, release new
versions, pull, compare and roll back.

## Once: sign in and link the project

```sh
odi login                 # opens the website: sign in (or sign up), then Allow
```

`odi login` opens the website in your browser. Sign in, or sign up if
you're new, and allow your computer; odi keeps a token of its own. Browse
what's published at [hub.odisssi.com](https://hub.odisssi.com). On a
machine with no browser (a server, CI), use an API key from
[your dashboard](https://dev.odisssi.com/dashboard#keys):
`odi login --token KEY`, or `ODI_TOKEN`. Another registry takes the token it
gives you: `odi login --registry URL --token TOKEN`.

Name the repository in the project's `odi.json`, so commands run in the
project don't need it:

```json
{ "repo": "acme/web" }
```

The first push to a new namespace claims it for your organization, and the
repository starts private (`odi push --public` makes it public).

## Publish

```sh
odi build
odi push
```

`odi push` publishes the build as the **file's version** and moves
`latest` to it. `odi build` takes that version, in order, from:

1. `"version"` in `odi.json`;
2. a version tag on the current commit (`git tag v1.4.2`);
3. the project's own version (`package.json`, `pyproject.toml`);
4. otherwise the commit (`3f2a91c4e7d1`), which isn't a version: then
   `odi push` only moves `latest`, or name one.

Name or pick the version yourself:

```sh
odi push acme/web:1.4.2          # this version
odi push --bump patch            # the newest version + 0.0.1 (minor, major too)
odi push acme/web:2.0.0-rc.1 --tag next   # a pre-release: latest stays where it is
odi push --tag staging           # also point staging at it
odi push --no-latest             # publish, but leave latest alone
```

Only the pieces the hub doesn't have are sent, so a new version of a large
app usually sends a few hundred KB.

**A published version doesn't change.** Pushing a different build to
`1.4.2` is refused, so `acme/web:1.4.2` means the same bytes forever and
for everyone. Bump the version instead, or, if you really must, `--force`.
Tags (`latest`, `staging`, anything that isn't a version) move freely.

### Several platforms

One version can hold a build per platform; a pull picks the machine's.

```sh
odi build --platform linux/amd64 -o web-amd64.odi
odi build --platform linux/arm64 -o web-arm64.odi
odi push acme/web:1.4.2 web-amd64.odi web-arm64.odi
```

From separate CI jobs, add each platform to the version as it's built:

```sh
odi push acme/web:1.4.2 web-arm64.odi --add
```

## What's there

```sh
odi versions acme/web            # (in the project: odi versions)
```

```
acme/web  private · 3 versions · 4 builds · 12 pulls · nextjs

VERSION     TAGS     PLATFORMS                SIZE     UPDATED
1.5.0-rc.1  next     linux/amd64              14.2 MB  2026-11-20 10:31
1.4.2       latest   linux/amd64 linux/arm64  14.1 MB  2026-11-19 16:02
1.4.1                linux/amd64 linux/arm64  14.1 MB  2026-11-12 09:40
```

`--all` also lists builds no tag points at.

## Pull and run

Every command that takes a name (`run`, `pull`, `inspect`, `cd`) takes a
version, a tag or a range:

| Name | Means |
|---|---|
| `acme/web` | `latest` |
| `acme/web:1.4.2` | exactly that version |
| `acme/web:1.4` | the newest 1.4.x |
| `acme/web:1` or `acme/web:^1.0` | the newest 1.x.y |
| `acme/web:~1.4.2` | the newest 1.4.x from 1.4.2 |
| `acme/web:staging` | wherever that tag points |
| `acme/web@sha256:…` | one build, by digest |

A tag with exactly that name wins over a range; pre-releases are only
picked by their full name.

```sh
odi pull acme/web:1.4            # odi: acme/web:1.4 is 1.4.2
odi run acme/web:1 --port 3000
```

## Coming next

Comparing two builds (`odi diff`), rolling back or promoting by moving a
tag (`odi tag acme/web:1.4.1 latest`), searching public repositories and
deleting builds are experimental for now: `odi help --all` lists them, and
[TODO.md](../TODO.md) what's planned.

## In CI

Set `ODI_TOKEN` to an API key (a CI secret) instead of `odi login`. For
example, a GitHub Actions job that publishes each `v*` git tag:

```yaml
on:
  push:
    tags: ["v*"]
jobs:
  publish:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
      - run: curl -fsSL https://github.com/manoj1604/odi/releases/latest/download/install.sh | sh
      - run: ~/.local/bin/odi build && ~/.local/bin/odi push
        env:
          ODI_TOKEN: ${{ secrets.ODI_TOKEN }}
```

The git tag `v1.4.2` becomes the build's version, so `odi push` publishes
`acme/web:1.4.2` and moves `latest`. To publish every push to `main` as a
moving tag instead: `odi push --tag main --no-latest`.
