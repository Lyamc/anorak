# Web UI vs GPUI client: before/after comparison

Branch `feature/gpui-frontend`. "Before" is the current web UI (Axum +
htmx HTML, `assets/*`). "After" is the native GPUI desktop client in
`anorak-gpui/`. Every number below was measured on 2026-09-25. Nothing is
estimated. Where a metric was not measured, the table says so and gives the
reason.

## Setup

| | |
|---|---|
| Client machine (both front-ends) | HYTE: Windows 11 Pro, i9-13900K (32 threads), RTX 3060 (driver 32.0.16.1047), display 1280×720 @ 60 Hz |
| Browser | Google Chrome 154.0.8037.57, **headed** (real window and GPU), fresh profile per run, cache disabled, driven by playwright-core over CDP |
| GPUI | `gpui = "=0.2.2"` from crates.io (the latest published release). Built with rustc 1.98.0 stable, `x86_64-pc-windows-msvc`, release profile `lto = "thin"`, `strip = true` |
| Server host | witherow.ca (NixOS), reached over the LAN at 192.168.0.101 |
| Web server under test | The **deployed master binary** (`/nix/store/jj3advb2…-anorak-0.1.0`) run by hand on port 8888 |
| GPUI server under test | The **branch build** (master plus `/api/query`) on port 8889 |
| Upstream data | Both servers pointed at the same mock Torznab server that serves a saved Lodestarr response for `one punch` (100 items, 166,841 bytes). rqbit was also mocked: it records POSTs and adds nothing. Production (`:9341`) and the real rqbit were not touched. |
| Window size | 1200×600 viewport (Chrome) and 1200×600 client area (GPUI) |
| Runs | 5 per front-end. The table shows the **median** (min–max in the notes) |

The branch server's `/query/` HTML is byte-identical to the deployed master
binary's for the same fixture, and `/`, `/style.css` are identical too, so
the web UI is unchanged on this branch.

## Results

| Metric | Web UI (before) | GPUI client (after) | Notes |
|---|---|---|---|
| **First-load transfer** | 82.4 KB on the wire (10 requests), 189.7 KB decoded | 12,354,048 B exe (11.8 MiB). As a download: 4.67 MB zip, 3.47 MB `tar.zst -19`, 3.17 MB `tar.xz` | Web: 31.6 KB comes from the anorak server uncompressed (it does not gzip), and 50.8 KB is gzip/zstd from CDNs (htmx 1.9.4 from unpkg, the Font Awesome kit and its CSS). The exe also needs the VC++ runtime (`VCRUNTIME140.dll`), which is usually already installed. |
| First search, extra one-time bytes | +159.0 KB (Font Awesome `fa-solid-900.woff2`, fetched when the first result icons appear) | 0 | |
| **Query response** (`one punch`, 100 rows) | 211,818 B wire (211,673 HTML + headers) | 55,258 B wire (55,122 JSON + headers) | Neither response is compressed by the server. With gzip (measured with bsdtar) they would be 17.1 KB (HTML) and 9.3 KB (JSON). |
| Repeat page load | Anorak's 31.6 KB is refetched every time (`Cache-Control: no-cache`, validators stripped); CDN files come from cache | n/a (the binary is already installed) | Not timed separately. |
| **Memory, results loaded** | Tab renderer: 103.8 MB working set | 59.9 MB working set (private 80.4 MB) | Web was sampled about 0.3–1 s after results, GPUI about 1 s after. |
| Memory after filter/sort + scroll script, peak | Tab renderer: 155.7 MB WS, peak 156.4 MB, private 103.5 MB | 62.2 MB WS, peak 62.2 MB | The web "tab renderer" is the largest of the 3 renderer processes; it is the only one that grew during the run. |
| Whole browser instance | 8 processes: 576 MB summed WS (double-counts shared pages), 340 MB summed private. The GPU process alone is 87 MB WS / 106 MB private | (single process, 68 threads) | Usually the browser is already open, so one more tab costs roughly the renderer plus some GPU-process memory, not the whole instance. |
| **Startup** | DOMContentLoaded 153.9 ms (the search form works from here because htmx loads synchronously in `<head>`). FCP 188 ms, `load` 199.9 ms. Chrome was already running. | Process launch to first painted frame: 367.9 ms (330–370 ms of that is inside the process, from `main()` to first paint). The search box is focused and usable at first frame. | Cold Chrome launch (playwright `launchPersistentContext`, to ready) took another 275 ms, measured separately. So cold browser plus page is about 475 ms, and warm browser plus page is about 154–200 ms. |
| **Search → 100 rows painted** | 124.1 ms (112.9–142.0) | 79.0 ms (71.1–88.3) | Both include the HTTP round trip to witherow with the mock upstream. GPUI's HTTP share was 68.8 ms (JSON parse 0.07–0.2 ms, state update < 0.1 ms, so about 9 ms render including the wait for vsync). GPUI renders only the visible rows (virtualized `uniform_list`, about 13 rows). The web UI lays out all 100. |
| Same search against live production | 1.5–2.0 s per `/query/` (3 curl runs to `:9341`, real Lodestarr) | not measured (production has no `/api/query`) | Live searches are dominated by the indexers, so the front-end difference is about 2–3 % of real search time. |
| **Filter + re-sort** (name contains "1080", sort Largest; then clear), change → next painted frame | 10.5 ms (8.5–20.7) | 11.3 ms (1.9–22.2) | Both are bound by the 60 Hz frame clock (up to 16.7 ms wait). The work itself was 2.05 ms median (max 4.7) in the web UI's synchronous JS handlers and 0.018 ms median (max 0.19) for the GPUI filter and sort. |
| **Scroll** 120 frames × 20 px, frame interval | mean 16.7, p95 16.8, max 17.0 ms | mean 16.7, p95 18.4, max 19.8 ms | Neither dropped a frame (a dropped frame shows as about 33 ms). The numbers are not directly comparable: web uses rAF timestamps, which are aligned to vsync, while GPUI uses wall-clock time at paint, which includes thread wake-up jitter. GPUI CPU time to build a frame (render + layout + paint, before GPU submit): mean 1.8 ms, p95 3.0 ms. The web equivalent was not measured (it needs a Chrome trace). |
| **Idle CPU** (20 s, results on screen, no input) | 10.9 ms/s across the whole instance (6.2–21.1). The tab renderer alone was 0.8–1.6 ms/s | 9.4 ms/s (4.7–24.2) | This is about 1 % of one core for both. Windows CPU time has 15.6 ms granularity, so over 20 s the resolution is about 0.8 ms/s. GPUI's Windows backend runs a vsync thread that wakes every frame, which is probably most of it (not profiled). |
| **Clean release build** (HYTE) | Server (master): 22.7 s, 175 crates compiled. Front-end: **no build step** | GPUI client: 126.8 s, 446 crates compiled. Incremental rebuild after touching `app.rs`: 25.8 s | Dependencies were already downloaded (`--offline`). The branch's server change adds no crates. |
| Dependency count | Server `Cargo.lock`: 250 packages. Front-end: 2 CDN libraries (htmx, Font Awesome kit) | `anorak-gpui/Cargo.lock`: 712 packages (all platforms). 446 are built on Windows, and `cargo tree` counts 537 for a Linux target. | The client is its own Cargo workspace, so the server's lock file and the Nix build are unaffected. |

## Method

- **Web** (`gpui-bench/webbench.mjs`): Chrome was launched headed with a
  fresh profile per run and cache disabled. Transfer sizes are summed from
  CDP `Network.loadingFinished.encodedDataLength` (wire, including headers)
  and `Network.dataReceived` (decoded). Startup numbers come from Navigation
  Timing and Paint Timing.
  - **Search:** `form.requestSubmit()`, then htmx's `htmx:afterSettle` on
    `#mainContent`, then `requestAnimationFrame`, then `setTimeout(0)`. The
    timer stops after the frame that contains the rows.
  - **Filter/sort:** set `#filterName` and `#sortPrimary` and dispatch the
    same `input` and `change` events the UI listens for, timed the same way.
  - **Scroll:** 120 rAF steps of `scrollBy(0, 20)`.
  - **Memory and CPU:** `Get-Process` on the processes whose command line
    contains the run's profile directory.
- **GPUI** (`gpui-bench/gpuibench.ps1` plus the client's `--bench` mode):
  the harness records wall-clock time just before `Start-Process`. The
  client appends JSON lines stamped at the **paint** of the frame that first
  shows each change (a zero-size `canvas` element painted last), so frame
  timings include layout and paint, and the present follows immediately.
  - **Startup:** first-frame paint time minus the launch time.
  - **Search:** Enter handler to paint of the frame with the rows. It
    includes the HTTP request, JSON decode, state update, the vsync wait and
    the draw.
  - **Filter/sort:** the same actions and values as the web run, five apply
    and five clear toggles per run.
  - **Scroll:** 120 frames with the list scroll offset moved 20 px per frame.
  - **Memory and CPU:** `Get-Process` working set, private bytes, peak
    working set and `TotalProcessorTime`.
- **Functional check** (`--selftest`): drives the same handlers the mouse
  and keyboard call, and logs state after each step. Screenshots were taken
  with `PrintWindow`. It checks that the Filter popover gives 72 of 100 rows
  for "1080" + min seeds 10 + min 500 MB, that Sort gives Largest then Newest,
  that header clicks toggle Title Z–A then A–Z, the select-all and
  indeterminate states, that Grab selected sends 3 POSTs and per-row Grab
  sends 1 (mock rqbit logged `torznab_category=5000/5040`), and that
  grabbed rows show ✓.
- **Build:** `cargo clean`, then `cargo build --release --offline`, timed
  with `Measure-Command` on HYTE for both crates.

## Caveats

- **This is not a live-indexer benchmark.** Both front-ends talk to servers
  on witherow that return a saved Lodestarr response, so the numbers isolate
  the front-end plus the LAN. Real searches take 1.5–2 s, mostly upstream.
- The two sides use different servers (deployed master on 8888, branch on
  8889). They run the same code apart from the added JSON route, on the same
  host and with the same fixture.
- **The display runs at 60 Hz**, so interaction and scroll numbers are
  frame-clock bound. A 144 Hz display would change them.
- HYTE was the user's live desktop, with other apps (e.g. Spotify) running,
  during the measurements. The GPUI window was not always in the foreground,
  though it still rendered at 60 fps. Expect some noise; that is why the table
  uses medians of 5 runs.
- Web memory is from Chrome's multi-process model and does not map 1:1 onto
  one native process. Summed working sets double-count shared pages. Private
  bytes are the better figure for "cost".
- GPUI draws only the visible rows, while the browser lays out all 100.
  That is how GPUI is meant to be used, but it favours GPUI on
  search-to-render and memory.
- GPUI numbers are **Windows / DirectX 11 only**. No Linux (Vulkan/Wayland)
  build or run was attempted, so no software-rendered numbers exist.
- Chrome's `performance.now()` is coarsened to about 0.1 ms, which limits
  the precision of the web compute times.
- Not measured:
  - Real mouse and keyboard input latency for either UI. Synthetic input
    would have taken over the user's cursor on a desktop in use.
  - Web frame build time (needs a Chrome trace).
  - GPU memory per process.
  - GPUI on a second display or at a HiDPI scale. (The wasm build was checked
    at 2× on `feature/gpui-wasm`, see below; native still was not.)

## Feature parity (GPUI client)

**Present:**
- Search on Enter.
- Filter popover (name contains, min seeds, min/max MB, Clear filters) and
  Sort popover (primary + then, the same 8 options, Clear sort), with active
  dots.
- Popovers close on Esc, outside click, toggle, or a new search. The buttons
  are disabled until results arrive and during a search.
- Column-header sort with the same rule as the web UI.
- Row checkboxes and tri-state select-all. Hidden rows are deselected.
- "N shown / M" and "N selected / Grabbed N".
- Per-row Grab and sequential Grab selected, sent to `/send-to-rqbit/` with
  the category.
- Already-added and grabbed rows show ✓; rows without a magnet show —.

**Different or missing:**
- The title cell does not expand in place. The full title is in a hover
  tooltip and in a strip below the table on click. Long titles are clipped,
  not ellipsized.
- Text labels replace the Font Awesome icons.
- The text field (adapted from GPUI's input example) has no undo, no
  double-click word select and no scrolling for text wider than the box.
- No Tab focus order between fields. (Fixed on `feature/gpui-wasm`: Tab and
  Shift-Tab now cycle the text fields on both targets, and Shift-Home/End
  select.)
- None of the browser extras: find-in-page, zoom, selecting and copying
  result text, a URL to share, or use from phones and other machines without
  installing a binary.
- The client needs a per-OS build and distribution.

**Better:**
- Search errors and failed grabs are shown, with Retry. The web UI silently
  keeps the old content on a 500.
- The query payload is 4× smaller.

## GPUI in the browser (WebAssembly), branch `feature/gpui-wasm`

`feature/gpui-wasm` ports `anorak-gpui` from `gpui 0.2.2` (crates.io) to
Zed's git GPUI (rev `933d8d93`), which has a web platform (`gpui_web`). The
same client source now builds for the desktop and for `wasm32-unknown-unknown`.
The anorak server serves the wasm bundle at **`/gpui/`**, so the browser
client needs no install. See `anorak-gpui/README.md` for the build.

- **Graphics backend.** `gpui_web` uses wgpu. It tries WebGPU first and falls
  back to WebGL2. **WebGPU needs a secure context** (https or localhost). On
  plain http over the LAN, e.g. `http://192.168.0.101:8889/gpui/`, there is
  no `navigator.gpu`, so it always runs on **WebGL2**. `?backend=webgl` or
  `?backend=webgpu` forces a backend. The WebGPU numbers below were taken by
  marking the origin as secure: Chrome's
  `--unsafely-treat-insecure-origin-as-secure` flag, and Firefox's
  `dom.securecontext.allowlist` pref.
- **Single-threaded build** (`gpui_web` with `default-features = false`). It
  does not need SharedArrayBuffer, so the server needs no COOP/COEP headers.
- **Fonts.** A browser canvas cannot use system fonts, so IBM Plex Sans
  Regular and SemiBold (OFL, 403 KB) are embedded. Native still uses Arial.
  CJK and emoji fall back to the browser's canvas fonts.
- **Caching.** `.wasm` and `.js` have content-hashed names and are sent with
  `Cache-Control: public, max-age=31536000, immutable`. Nothing else in
  `no_stale_cache` changed.

### Results (2026-09-25)

Same machine, window size, fixture servers and in-app `bench` script as
above. The wasm client logs the same events as the native `--bench` mode to
`window.__anorakBench`, stamped at the paint of the frame that shows the
change. **n** is the number of runs behind each median: 5 runs everywhere
except Firefox WebGPU. That column is the median of 4, because the 5th run
was interrupted and thrown away. Ranges are min–max.

| Metric | Web UI (from the table above, n=5) | GPUI native (Zed git GPUI, re-measured, n=5) | GPUI WASM, Chrome 154 (n=5) | GPUI WASM, Firefox 147 (WebGL2 n=5, WebGPU n=4) |
|---|---|---|---|---|
| **Download / first-load transfer** | 82.4 KB wire (10 requests), 189.7 KB decoded | exe 11,865,600 B (0.2.2 build: 12,354,048 B) | **7.45 MB on the wire**, sent uncompressed (wasm 7,287,534 B + js 160,411 B + html 1,290 B). 7.52 MB including favicon and `/api/query`. | same bundle: wasm 7,296,080 B transferred |
| Bundle if the server compressed it | – | – | wasm **2.03 MB brotli-11** / 2.84 MB gzip-9 / 2.17 MB zstd-19; js 19 KB br / 23 KB gz | same |
| **Repeat visit** | Anorak's 31.6 KB is refetched every time (not timed) | n/a | 1,440 B on the wire (only `index.html`; js and wasm come from cache) | wasm from cache (0 B) |
| **Startup to first painted frame** | DCL 153.9 ms, FCP 188, load 199.9 | Launch → first frame **205.4 ms** (202.8–267.5), 184.0 in-process (0.2.2: 367.9) | **Cold cache:** WebGL2 **834.8 ms** from navigation start (781–933); WebGPU **489.6** (382–715). **Warm cache:** WebGL2 **89.3** (76.8–135.5); WebGPU **180.0** (125–196) | **Cold:** WebGL2 **797** (783–1058); WebGPU **574** (494–707). **Warm:** WebGL2 **627** (585–643); WebGPU **263** (260–270) |
| └ of which wasm start → first frame | – | – | Cold: WebGL2 502.7 (500.0–504.8), WebGPU 101.5. Warm: WebGL2 46.6, WebGPU 89.4 | Cold: WebGL2 507, WebGPU 239. Warm: WebGL2 478, WebGPU 156 |
| **Search → 100 rows painted** | 124.1 ms | **75.8** (72.0–105.4), HTTP 63.1 (0.2.2: 79.0) | WebGL2 **77.1**, WebGPU **74.3** (HTTP ≈ 62) | WebGL2 **86** (83–167), WebGPU **119** (85–162). Bimodal: each run was either ~85 or ~160 |
| **Memory, results loaded** | Tab renderer 103.8 MB WS | **55.6 MB WS** (private 82.8) (0.2.2: 59.9) | Renderer: WebGL2 267.8 MB WS (private 170.6); WebGPU 241.8 (private 138.8) | Content process: WebGL2 101.7 MB WS; WebGPU 99.7 |
| **Memory after filter/sort + scroll** | Renderer 155.7 MB WS (private 103.5); GPU process 87 WS / 106 private | 58.4 MB WS (private 85.3), peak 58.8 | **WebGL2:** renderer 276.3 WS / 174.3 private (peak 306.7); GPU process 91.9 WS / 135.1 private. **WebGPU:** renderer 251.1 / 141.6 (peak 281.2); GPU 157.4 / 188.1 | **WebGL2:** content 108.7 WS / 79.5 private; GPU process 111.5 WS / 272.4 private. **WebGPU:** content 109.9 / 80.1; GPU 178.3 / 331.7 |
| Whole browser instance | 8 processes, 576 MB summed WS, 340 MB private | single process, 68 threads | 8 processes. WebGL2: 568.5 MB WS / 374.1 private. WebGPU: 608.4 / 396.4 | 11 processes. WebGL2: 795 MB WS / 761 private. WebGPU: 867 / 824 (the Marionette/BiDi automation adds to this) |
| **Filter + re-sort → frame** (median of all toggles) | 10.5 ms (compute 2.05) | 13.2 (1.6–22.6), compute ~0.0 (0.2.2: 11.3) | WebGL2 3.3 (2–18.6), WebGPU 2.4 (1.9–14.8), compute 0.0 | WebGL2 5 (2–13), WebGPU 4 (2–7) |
| **Scroll** 120 × 20 px, frame interval mean / p95 / max | 16.7 / 16.8 / 17.0 | 16.7 / 18.3 / 19.5 | WebGL2 16.6 / 17.6 / **34.6**; WebGPU 16.6 / 17.6 / **35.6**. **Every run had one ~34 ms frame (one dropped frame)** | WebGL2 16.5 / 18 / 20; WebGPU 16.6 / 18 / 20.5. No dropped frames |
| Frame build (CPU, before submit) mean / p95 | not measured | 2.2 / 3.3 | WebGL2 2.2 / 3.0; WebGPU 1.6 / 2.1 | WebGL2 2.5 / 4; WebGPU 2.4 / 4 |
| **Idle CPU** (20 s, whole instance) | 10.9 ms/s (renderer ~1) | 6.2 ms/s (3.9–15.6) (0.2.2: 9.4) | WebGL2 23.4 (renderer 11.7, GPU 3.1); WebGPU 15.6 (renderer 3.9, GPU 1.6) | WebGL2 69.5 (30–106); WebGPU 56.6 (32–61). Noisy, includes the automation agent |
| **Clean release build** (HYTE) | no front-end build step | 105.2 s, 312 crates; incremental 13.6 s (0.2.2: 126.8 s / 446 crates / 25.8 s) | `trunk build --release`: **156.2 s** (cargo 137 s, 337 crates, then ~19 s wasm-bindgen + wasm-opt). Incremental after touching `app.rs`: 66.3 s | same bundle |
| Dependencies | 2 CDN libraries | `Cargo.lock` 669 packages (0.2.2: 712) | same lock file | |

Notes on the table:
- **Cold vs warm startup.** Cold cache is dominated by fetching 7.3 MB of
  uncompressed wasm (responseEnd about 320–380 ms over the LAN) plus compile
  and GPU setup. On WebGL2, wasm start → first frame is a near-constant
  ~500 ms (500.0–504.8 in Chrome cold; 473–510 in Firefox cold and warm). That
  coincides with the "Failed to poll device during resize: Timeout" warning,
  which comes from a blocking `device.poll(Wait)` in `gpui_wgpu`'s resize path.
  So it looks like a fixed stall rather than real work, and probably
  fixable upstream (not confirmed with a profile). WebGPU does not have it.
  On a warm visit Chrome reaches the first frame in about 90 ms (WebGL2),
  faster than the native exe's 205 ms. Firefox does not get that speed-up in
  this setup (warm 263 ms on WebGPU, 627 ms on WebGL2; not investigated).
- **Search → render** is the same in all columns, about 62 ms of it being the
  HTTP round trip, because the work after the response is under 15 ms
  everywhere.
- **Filter/sort in the browser is measured to the paint inside the next
  `requestAnimationFrame`,** which does not include the compositor's present.
  Native measures to paint just before present plus the DXGI vsync wait. So
  3 ms (wasm) vs 13 ms (native) is not a like-for-like win. Both are
  frame-clock bound.
- **Memory:** a GPUI tab costs about 2× the web UI's renderer in Chrome
  (174 vs 104 MB private on WebGL2), and more in the GPU process. The wasm heap
  holds the font atlas, the embedded fonts, the glyph cache and the wgpu
  state. Firefox's content process is much smaller (80 MB private), but its
  GPU process holds more (272–332 MB private).
- **The dropped frame in Chrome** happened once per 120-frame scroll, on both
  backends, and not in Firefox or native. It was not investigated further.
- Chrome was measured on build `4f86613` (the commit before the key-binding
  fixes). Firefox and the sizes table below use the final build `e06f127`,
  which is 8 KB larger and differs only in key bindings and the tab stop.

Wasm bundle sizes, final build (`docs/gpui-bench/sizes-wasm.txt`):

| File | raw | gzip-9 | brotli-11 | zstd-19 |
|---|---|---|---|---|
| `anorak-gpui-*_bg.wasm` | 7,295,780 | 2,836,522 | 2,032,324 | 2,174,900 |
| `anorak-gpui-*.js` | 160,411 | 23,003 | 19,252 | 20,696 |
| `index.html` | 1,286 | 764 | 548 | 761 |
| of which: IBM Plex Sans Regular + SemiBold (embedded in the wasm) | 403,132 | 179,826 | 145,701 | 158,089 |

The raw rustc output before `wasm-opt -Os` was 9,423,220 B. The anorak server
does not compress any response, so today the browser downloads the full
7.45 MB. Adding compression (e.g. `tower-http` `CompressionLayer`, or
precompressed `.br` files served by `ServeDir`) would cut that to about
2.05 MB. It was left out to keep the server change small.

### Method (wasm)

- `gpui-bench/wasmbench.mjs` (Chrome): headed Chrome via playwright-core.
  Each run gets a fresh profile, loads `about:blank` for 1 s so Chrome is
  warm, then opens `/gpui/?bench=1`.
  - **Cold:** first load with an empty cache.
  - **Idle CPU:** 20 s with results on screen and no input.
  - **Warm:** `about:blank`, then the same URL again in the same profile.
  - **Memory and CPU:** `Get-Process` on every process whose command line
    names the run's profile. Renderer and GPU-process figures are listed
    separately.
  - **Transfer:** from CDP `Network` events.
  - **First frame:** `performance.now()` at the paint of the first frame,
    which is relative to navigation start. "wasm start → frame" is measured
    from the wasm entry point.
- `gpui-bench/ffbench.mjs` (Firefox): puppeteer-core over WebDriver BiDi
  against the stock Firefox 147 install, with the same page flow and bench
  events. Memory is the Firefox process tree (the main process and its
  descendants). Firefox's timer resolution is 1 ms.
- `gpui-bench/parity.mjs`, `cliptest.mjs`: functional checks with synthetic
  CDP input (keyboard, mouse, wheel, IME composition, `insertText`),
  screenshots, and the `/api/query` request as the oracle for text-field
  contents.
- `gpui-bench/s_wasm.sh`: starts the branch server plus the fixture mocks and
  serves the bundle. `sizes.sh`: compression sizes.
- Raw results: `results-gpui-zedgit.json`,
  `results-wasm-{chrome,firefox}-{webgl,webgpu}.json`.

### Caveats (wasm)

- HYTE crashed twice during this work (a driver under development, unrelated
  to anorak). Every run interrupted by a crash, or by hand, was discarded,
  and builds were checked or redone afterwards. The remaining runs are
  complete.
- As above: 60 Hz display, synthetic input only, the user's desktop in use,
  and one machine (RTX 3060; WebGL2 runs on ANGLE D3D11 in Chrome). Other
  automation was also using HYTE around the same time, so expect noise,
  especially in idle CPU.
- Firefox numbers come from an automated browser (the BiDi remote agent is
  active), which inflates its idle CPU and main-process memory.
- Chrome prints two warnings on WebGL2: "Dual-source blending not available
  → subpixel text antialiasing disabled" (text is grayscale-AA in the
  browser), and "Failed to poll device during resize: Timeout" (harmless;
  rendering continues).

### Browser parity (wasm client)

All of this was checked with synthetic input in Chrome 154 on WebGL2. Layout
and scrolling were also checked on Chrome WebGPU. Typing, non-ASCII,
copy/paste, Tab, tooltips, wheel scroll and the Filter popover were also
checked in Firefox 147 on both WebGL2 and WebGPU. IME, drag-select and Grab
were tested in Chrome only.

**Works:**
- Typing, Home/End/Backspace, select-all replace, and non-ASCII
  ("naïve café – ✓").
- IME composition ("わんぱん" shown in the field while composing, commits
  "ワンパンマン"). Emoji via `insertText`. CJK titles render through the canvas
  fallback font.
- Copy, cut and paste over plain http, through the browser's native clipboard.
- Mouse drag-select, then typing over the selection.
- Tab / Shift-Tab between fields (DOM focus stays in GPUI's hidden textarea).
- Row tooltips, the Filter and Sort popovers (Esc and outside click close
  them), per-row Grab, and Grab selected (3 POSTs logged by the mock rqbit
  with categories 5000/5040/5000), with the ✓ state afterwards.
- Wheel scrolling (the list clips correctly under the header), window resize
  (the canvas follows the viewport), and HiDPI: at a real device scale factor
  of 2 the canvas backing store is 2× its CSS size and text is sharp.

**Gaps:**
- **Double-click word select** does not work (same as native; the text field
  comes from GPUI's input example).
- Clicking the Filter button does not focus the popover's first field. Typing
  right after opening it goes nowhere until you click or Tab into a field
  (same as native).
- Ctrl+C pressed in the same event-loop tick as Ctrl+A copies the previous
  selection, because gpui_web syncs the selection into its hidden textarea
  asynchronously. A human never hits this.
- Result text cannot be selected, and there is no find-in-page or
  screen-reader tree: it is all one canvas.
- Under Chrome's emulated device scale factor (DevTools device mode /
  `Emulation.setDeviceMetricsOverride`) the canvas stays at 1× and looks
  blurry. A real HiDPI display or `--force-device-scale-factor` is fine.
- **Upstream `gpui_web` bug:** `write_to_clipboard` calls
  `navigator.clipboard.writeText` without checking that it exists. On plain
  http it does not, the JS exception leaves a `RefCell` borrowed, and all
  later input is dead. The client therefore binds Ctrl+C/X/V only on native
  and lets the browser's own clipboard handling work in the browser.
- `?server=` pointing at another origin needs CORS on that server. The
  default is the page's own origin.
