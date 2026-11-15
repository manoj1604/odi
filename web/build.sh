#!/bin/sh
# Builds odi_web.wasm (crates/web) into this folder: the runner is then these
# four files, to serve from anywhere (odi web serves them too, built in).
set -e
cd "$(dirname "$0")/.."
cargo build -p odi-web --release --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/odi_web.wasm web/odi_web.wasm
ls -l web/odi_web.wasm
