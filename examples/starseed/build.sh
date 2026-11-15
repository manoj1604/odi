#!/bin/sh
# Builds Starseed into starseed.odi: the Rust core to WebAssembly
# (site/core.wasm), the music (site/audio/, made once), then odi build.
#   examples/starseed/build.sh [ODI]      then: odi web starseed.odi
set -e
cd "$(dirname "$0")"
ODI=${1:-odi}
CARGO=$(command -v cargo || echo "$HOME/.cargo/bin/cargo")
(cd core && "$CARGO" build --release --target wasm32-unknown-unknown)
cp core/target/wasm32-unknown-unknown/release/starseed_core.wasm site/core.wasm
python3 gen_audio.py site/audio
"$ODI" build site -o starseed.odi --no-build
