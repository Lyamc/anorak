#!/bin/sh
# Precompress the Trunk output for the Anorak server, which serves
# FILE.br / FILE.gz (Content-Encoding br / gzip) instead of FILE when the
# browser accepts it (ServeDir::precompressed_br/_gzip in src/main.rs).
# Run after `trunk build --release`:  sh precompress.sh [dist]
# Needs `brotli` and `gzip` (on NixOS: nix-shell -p brotli --run 'sh precompress.sh').
set -eu
DIST=${1:-dist}
for f in "$DIST"/*.wasm "$DIST"/*.js "$DIST"/*.html; do
    [ -f "$f" ] || continue
    brotli -f -q 11 -o "$f.br" "$f"
    gzip -f -9 -n -c "$f" > "$f.gz"
done
ls -l "$DIST"
