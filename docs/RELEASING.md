# Releasing

1. Make sure `main` is green in CI.
2. Set the version in `Cargo.toml` (`[workspace.package] version`) and run
   `cargo build` so `Cargo.lock` follows.
3. In `CHANGELOG.md`, change `## [X.Y.Z] - unreleased` to today's date.
4. Rebuild generated files and check nothing changed unexpectedly:
   `sh web/build.sh`, `sh docs/gen-cli.sh`.
5. Commit ("odi X.Y.Z"), tag and push:

   ```sh
   git tag -a vX.Y.Z -m "odi X.Y.Z"
   git push origin main vX.Y.Z
   ```

6. The `release` workflow builds odi for Linux (static, musl) and macOS
   (x86_64 and arm64) and the browser runner, and makes a **draft** GitHub
   release with them, `SHA256SUMS`, `install.sh` and the changelog's notes.
   It refuses a tag that doesn't match `Cargo.toml`'s version.
7. Check the draft: download a build, run `odi version`, try
   `install.sh` on a clean machine. Then publish it.
8. Optional: publish the library, `cargo publish -p odi-format`, then
   `cargo publish -p odi`.

Tags with a `-` (`v0.2.0-rc.1`) are marked as pre-releases.
