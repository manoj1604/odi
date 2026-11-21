# odi docs

New here? Start with **[Getting started](getting-started.md)**. It takes
about five minutes.

## I want to…

| …do this | Read |
|---|---|
| Build my project into an `.odi` file, run it, `cd` into it | [Getting started](getting-started.md) |
| Run it on a VM or server with nothing installed | [Getting started](getting-started.md#run-it-where-odi-isnt-installed) |
| Publish versions and pull them | [Publishing on the hub](hub.md) |
| Look up a command or flag | [Command reference](cli.md), or `odi help COMMAND` |
| Set what a project is and how it builds | [odi.json](odi-json.md) |
| Change settings, folders or environment variables | [Configuration](configuration.md) |

## Experimental

These work today and may change before they're part of the core (see
[TODO.md](../TODO.md); `odi help --all` lists their commands):

- [Parts and sets](sets.md): one runtime shared by many apps, themes and
  mods layered over an app, services beside it.
- [Tools for AI agents](agents.md): `odi exec`, `odi mcp`, `odi attach`.
- [Browser apps and games](browser.md): `odi web`, offline play.
- [Signing and trust](signing.md): `odi key`, `odi sign`, `--trust`.
- [Registries](registry.md): fast starts (`odi profile`, `odi fill`) and
  the protocol, for running your own.
- [The format specification](../SPEC.md): for anyone writing a reader,
  runner or registry.

## Words you'll see

- **ODI file** (`.odi`): one file holding your build output plus a small
  manifest that says how to start it.
- **Manifest**: the JSON inside the file listing processes, environment
  variables and static files. `odi inspect` shows it.
- **Digest**: the file's SHA-256 fingerprint (`sha256:…`). The same digest
  always means exactly the same content.
- **Hub / registry**: where files are published, as `namespace/name:tag`,
  for example `acme/web:1.4.2`. The default is `https://hub.odisssi.com`.
- **Tag**: a name for a published build. Versions like `1.4.2` never
  change; tags like `latest` move.

Maintainers: see [Releasing](RELEASING.md).
