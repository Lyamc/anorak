# Size-optimized web build (dist/) on Windows; see build-web.sh.
# Precompress on the server host afterwards with precompress.sh.
# Extra arguments go to trunk, e.g. .\build-web.ps1 --dist C:\somewhere\dist
# (No $ErrorActionPreference = "Stop": Windows PowerShell turns trunk's stderr
# into a terminating error when the output is redirected.)
Set-Location $PSScriptRoot
$env:RUSTFLAGS = '--cfg getrandom_backend="wasm_js" -Zunstable-options -Cpanic=immediate-abort'
$env:CARGO_UNSTABLE_BUILD_STD = "std,panic_abort"
$env:CARGO_UNSTABLE_BUILD_STD_FEATURES = "optimize_for_size"
try { trunk build --release @args; $code = $LASTEXITCODE } finally {
    Remove-Item Env:RUSTFLAGS, Env:CARGO_UNSTABLE_BUILD_STD, Env:CARGO_UNSTABLE_BUILD_STD_FEATURES -ErrorAction SilentlyContinue
}
exit $code
