#!/bin/sh
# Index Shakespeare and bundle its search page into target/shakespeare/site/, then score both query sets. Run
# ../../build.sh first, or after changing the engine; this script only uses the search-cli it built.
set -eu
cd "$(dirname "$0")"
cli=../../target/release/search-cli
out=../../target/shakespeare
mkdir -p "$out"

"$cli" index records.tsv --table forms.tsv --table suffixes.tsv --table synonyms.tsv --table aliases.tsv --table stop.tsv > "$out/index.bin"
rm -rf "$out/site"
"$cli" bundle "$out/index.bin" -o "$out/site" --name Shakespeare \
  --about "Every play, sonnet and poem. Type a line the way you remember it." \
  --credit "Text: the PlayShakespeare.com editions, edition 5.4, under the GNU Free Documentation License 1.3 or later, fdl-1.3.txt. https://github.com/severdia/PlayShakespeare.com-XML" \
  --file fdl-1.3.txt
# The page inflates a gzipped index itself, so it ships compressed whatever the host does.
gzip -9nc "$out/site/index.bin" > "$out/site/index.bin.gz" && mv "$out/site/index.bin.gz" "$out/site/index.bin"
ls -l "$out/site/index.bin" "$out/site/pkg/search_wasm_bg.wasm"

"$cli" eval "$out/index.bin" tuned.tsv heldout.tsv | grep -E 'records|rank 1'
