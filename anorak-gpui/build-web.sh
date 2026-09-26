#!/bin/sh
# Size-optimized web build (dist/), then precompress for the server.
# Plain `trunk build --release` also works; this adds, on nightly:
#   -Zbuild-std with optimize_for_size, and panic=immediate-abort
#   (panics trap without a message; the wasm is ~10 % smaller).
set -eu
cd "$(dirname "$0")"
export RUSTFLAGS='--cfg getrandom_backend="wasm_js" -Zunstable-options -Cpanic=immediate-abort'
export CARGO_UNSTABLE_BUILD_STD=std,panic_abort
export CARGO_UNSTABLE_BUILD_STD_FEATURES=optimize_for_size
trunk build --release "$@"
if command -v brotli >/dev/null 2>&1; then sh precompress.sh dist; fi
