# anorak-gpui

Front-end for the Anorak server built with [GPUI](https://gpui.rs) (Zed's
GPU-accelerated Rust UI framework). The same code builds as a native desktop
app and as WebAssembly for the browser. It talks to a running Anorak server
over HTTP; it does not replace the web UI.

- GPUI comes from Zed's git repository pinned to one commit (see
  `Cargo.toml`), because the web platform (`crates/gpui_web`,
  zed-industries/zed#50228) and `gpui_platform` are not on crates.io.
  Native builds use `gpui_platform`; wasm uses `gpui_web` directly, single
  threaded (no `SharedArrayBuffer`, COOP/COEP headers or `build-std`).
- Server API used: `GET /api/query?search_term=...` (JSON) and the existing
  `POST /send-to-rqbit/` (form: `magnet`, `category`).

## Build and run (native)

```sh
cd anorak-gpui
cargo build --release
./target/release/anorak-gpui --server http://192.168.0.101:9341
```

## Build and run (browser)

```sh
rustup target add wasm32-unknown-unknown --toolchain nightly
cargo install trunk            # 0.21.x; it fetches wasm-bindgen and wasm-opt
cd anorak-gpui
trunk build --release          # -> dist/ (index.html, .js, _bg.wasm)
# or, smallest bundle (build-std + panic=immediate-abort), then .br/.gz:
sh build-web.sh                # Windows: .\build-web.ps1, then precompress.sh on the server host
```

`precompress.sh [dist]` writes `FILE.br` (brotli -q 11) and `FILE.gz` next
to each file. The server then sends those with `Content-Encoding` to
browsers that accept them. Without them it serves the plain files. The
full build is 5.6 MB of wasm, 2.26 MB gzip and 1.69 MB brotli. Browsers only
ask for brotli over https.

The Anorak server serves `anorak-gpui/dist` at `/gpui/` (override the
directory with `ANORAK_GPUI_DIST`), so the page calls `/api/query` and
`/send-to-rqbit/` on its own origin with no CORS. Open
`http://SERVER:PORT/gpui/`. Query parameters:

- `backend=webgpu` or `backend=webgl` forces a renderer. The default tries
  WebGPU and falls back to WebGL2. WebGPU needs a secure context (https or
  localhost), so over plain `http://192.168.0.101:…` browsers get WebGL2.
- `server=URL` talks to another Anorak server (it then needs CORS).
- `bench=1` or `selftest=1` (plus `term=`) run the measurement scripts; the
  JSON lines go to `window.__anorakBench` and the console.

The wasm build needs nightly (`gpui_web` enables `parking_lot`'s `nightly`
feature) and the `getrandom_backend="wasm_js"` cfg from `.cargo/config.toml`.
Trunk uses the `wasm-release` profile (`opt-level = "z"`, fat LTO, one
codegen unit, `panic = "abort"`) and runs `wasm-opt -Oz`. `build-web.sh`
also rebuilds std for size and uses `-Cpanic=immediate-abort`, so a panic
traps with no message. For a debuggable bundle, use plain `trunk build`.
`vendor/gpui_wgpu` is a patched copy of Zed's crate that creates render
pipelines lazily on wasm, which avoids a ~440 ms WebGL2 startup stall
(`vendor/gpui_wgpu/PATCHES.md`). Bump it together with the zed rev. If you set
`RUSTFLAGS` it replaces the config's rustflags, so include
`--cfg getrandom_backend="wasm_js"` yourself.

The browser platform has no system fonts, so the wasm embeds IBM Plex Sans
Regular and SemiBold (`assets/fonts`, SIL OFL 1.1, subset to 289 KB raw,
106 KB brotli; SemiBold has no Cyrillic/Greek, which fall back to Regular,
see `assets/fonts/ibm-plex-sans/README.md`). CJK graphemes the font lacks (some titles) are
drawn by gpui_web's canvas fallback with the browser's fonts.

The server URL comes from `--server URL`, then `$ANORAK_SERVER`, and falls back
to `http://192.168.0.101:9341`. `--size 1280x800` sets the initial window size.

This crate is its own Cargo workspace (it is not a member of the server's).
That keeps GPUI's dependency tree (669 packages in `Cargo.lock`, about 310
built on Windows) out of the server's
`Cargo.lock`, which `nix/package.nix` uses to build the deployed server.

`rust-toolchain.toml` says `nightly` (for the wasm build). Native builds also
work on stable. On a Windows host whose rustup default host is GNU, build with
`cargo +stable-x86_64-pc-windows-msvc build --release` because GPUI's
DirectX backend expects MSVC. It needs the Windows SDK
(`fxc.exe`) for shader compilation. On Linux it needs Vulkan, Wayland and/or
X11, and xkbcommon development libraries.

## Features (parity with the web UI)

- Search box: Enter submits. Results come back in the server's default order
  (seeders, then peers).
- **Filter** popover: name contains, min seeds, min/max size (MB), and Clear
  filters. A dot on the button shows when filters are active.
- **Sort** popover: primary and "then" sort over the web UI's eight options,
  and Clear sort. Clicking a column header sorts by that column (the same
  column flips direction; a new column starts descending), as in the web UI.
- The popovers close on Escape, on an outside click, on toggle, and when a new
  search starts. The buttons are disabled until results are loaded.
- Results table: per-row checkbox and a header select-all (checked,
  indeterminate, or clear). Hidden rows are deselected. Shows "N shown / M"
  and "N selected".
- Per-row **Grab** and **Grab selected** (sequential) send a `POST` to
  `/send-to-rqbit/` with the Torznab category. Rows that are already in rqbit
  or have been grabbed show ✓. Rows with no magnet show —. A failed grab
  shows Retry and the error in a tooltip.
- A hover tooltip shows the full title. Clicking the name shows it in full
  below the table (the web UI expands the cell in place).

## Measurement modes

- `--bench LOG` runs a scripted search, 5 filter/re-sort toggles and a
  120-frame scroll, then appends JSON lines with in-process timings taken at
  the paint of the frame that shows the change.
- `--selftest LOG` drives the same handlers the UI uses (popovers, filters,
  sort, select-all, grab) and logs state after each step.
- In the browser the same modes are `?bench=1` and `?selftest=1`.

See `docs/gpui-comparison.md` in the repo root for the results.
