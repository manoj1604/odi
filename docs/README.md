# odi docs

New here? Start with **[Getting started](getting-started.md)**. It takes
about five minutes.

## I want to…

| …do this | Read |
|---|---|
| Build my project into an `.odi` file and run it | [Getting started](getting-started.md) |
| Publish versions, pull them, roll back | [Publishing on the hub](hub.md) |
| Look up a command or flag | [Command reference](cli.md), or `odi help COMMAND` |
| Share one runtime between apps, add themes or mods | [Parts and sets](sets.md) |
| Give an AI agent a tool (MCP) | [Tools for AI agents](agents.md) |
| Play a browser game or app from a file, offline too | [Browser apps and games](browser.md) |
| Sign files and only run what I trust | [Signing and trust](signing.md) |
| Run my own registry, or make starts faster | [Registries](registry.md) |
| Change settings, folders or environment variables | [Configuration](configuration.md) |
| Write my own reader, runner or registry | [The format specification](../SPEC.md) |

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
- **Set**: a tiny ODI file that lists other ODI files (parts) to run
  together, such as an app plus a shared Node.js runtime.

Maintainers: see [Releasing](RELEASING.md).
