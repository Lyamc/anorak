# Size-optimized web build (dist/) on Windows; see build-web.sh.
# Precompress on the server host afterwards with precompress.sh.
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot
$env:RUSTFLAGS = '--cfg getrandom_backend="wasm_js" -Zunstable-options -Cpanic=immediate-abort'
$env:CARGO_UNSTABLE_BUILD_STD = "std,panic_abort"
$env:CARGO_UNSTABLE_BUILD_STD_FEATURES = "optimize_for_size"
try { trunk build --release @args } finally {
    Remove-Item Env:RUSTFLAGS, Env:CARGO_UNSTABLE_BUILD_STD, Env:CARGO_UNSTABLE_BUILD_STD_FEATURES -ErrorAction SilentlyContinue
}
