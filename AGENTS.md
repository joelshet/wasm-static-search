# Working in wasm-static-search

`README.md` covers what the engine does, the record and table formats, how ranking works, and where each part lives.

- `./build.sh` tests the engine, compiles it to WebAssembly into `web/pkg/`, builds `search-cli`, and builds and scores the Shakespeare example. It needs Rust with the `wasm32-unknown-unknown` target, plus `wasm-pack`.
- `web/pkg/` is committed so `cargo build --release` alone gives a working `search-cli`. After changing `crates/core/` or `crates/wasm/`, run `./build.sh` and commit the rebuilt `web/pkg/` with the change.
- Check ranking changes with `./examples/shakespeare/build.sh`, which scores the tuned and held-out query sets. Never tune against `heldout.tsv`. The README's Numbers section has the current scores; update it when they move.
- The code is MIT. The Shakespeare text is under the GNU FDL 1.3 (`examples/shakespeare/fdl-1.3.txt`), and `examples/shakespeare/raw/` is downloaded by `prepare.py` and never committed.
