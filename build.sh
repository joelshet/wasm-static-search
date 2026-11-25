#!/bin/sh
# Build everything from source: test the engine, compile it to WebAssembly for the page in web/, build search-cli
# (which carries that page inside it), and build the Shakespeare example. Needs Rust with the
# wasm32-unknown-unknown target, plus wasm-pack.
set -eu
cd "$(dirname "$0")"

cargo test --release --quiet
wasm-pack build crates/wasm --release --target web --out-dir "$PWD/web/pkg" --no-typescript --no-pack
rm -f web/pkg/.gitignore
cargo build --release --quiet
./examples/shakespeare/build.sh
