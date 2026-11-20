# Contributing to odi

Thanks for helping. Bug reports, ideas, docs fixes and code are all
welcome.

## Before you start

- **Bugs:** open an issue with what you ran, what happened and what you
  expected. `odi version` and your OS help.
- **Bigger changes** (a new command, a format change, a new language for
  `odi build`): open an issue first, so we can agree on the shape before
  you spend time on it.
- **Format changes** need a change to [SPEC.md](SPEC.md) in the same pull
  request. Files that already exist must keep opening, and their digests
  must not change (`crates/format/tests/go_digests.rs` guards this).
- **Security problems:** please don't open a public issue; see
  [SECURITY.md](SECURITY.md).

## Where things are

- [`crates/format`](crates/format) (`odi-format`): the Rust library that
  reads and writes ODI files, with every check a reader must make.
- [`crates/odi`](crates/odi): the `odi` command.
- [`crates/web`](crates/web) and [`web/`](web): the browser runner (the
  reader compiled to WebAssembly, plus a page and service worker).
- [`examples/starseed`](examples/starseed): a small WebGPU game packed in
  one ODI file.
- [`tests/`](tests): end-to-end tests; [`bench/`](bench): timing runs and
  a mock registry.
- [`docs/`](docs): user docs; [`SPEC.md`](SPEC.md): the format.

## Building and testing

You need a recent stable Rust. Then:

```sh
cargo build                                    # target/debug/odi
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

Some things need Linux: reading files from a registry as they're read
(FUSE, `apt install fuse3`) and `odi profile`. On a Mac, a Linux VM such
as [Lima](https://lima-vm.io) works well.

The browser runner's WebAssembly reader:

```sh
rustup target add wasm32-unknown-unknown
cargo test -p odi-web
sh web/build.sh                                # rebuilds web/odi_web.wasm
```

`web/odi_web.wasm` is checked in, because `odi web` builds it into the
binary: rebuild and commit it when you change `crates/web` or
`crates/format`'s reading code.

If you change a command's flags or help text, regenerate the command
reference: `sh docs/gen-cli.sh` (CI checks it's up to date).

## Pull requests

- Keep them focused: one change per pull request.
- Add or update tests for what you change.
- Update the docs (`docs/`, README) when behaviour changes, and add a line
  to [CHANGELOG.md](CHANGELOG.md) under "unreleased".
- CI must pass: tests on Linux and macOS, clippy, formatting.

## Licence

By contributing, you agree that your contributions are licensed under the
[Apache License 2.0](LICENSE), the project's licence (Apache 2.0, §5). There
is no CLA.

## Conduct

Everyone taking part is expected to follow the
[code of conduct](CODE_OF_CONDUCT.md).
