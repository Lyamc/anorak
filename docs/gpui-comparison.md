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
  - GPUI on a second display or at a HiDPI scale.

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
- No Tab focus order between fields.
- None of the browser extras: find-in-page, zoom, selecting and copying
  result text, a URL to share, or use from phones and other machines without
  installing a binary.
- The client needs a per-OS build and distribution.

**Better:**
- Search errors and failed grabs are shown, with Retry. The web UI silently
  keeps the old content on a 500.
- The query payload is 4× smaller.
