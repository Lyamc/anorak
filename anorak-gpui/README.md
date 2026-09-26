# anorak-gpui

Native desktop front-end for the Anorak server, built with
[GPUI](https://gpui.rs) (Zed's GPU-accelerated Rust UI framework). It talks
to a running Anorak server over HTTP; it does not replace the web UI.

- GPUI: `gpui = "=0.2.2"` from crates.io (the latest published release;
  Zed's `main` has since split out `gpui_platform`, which is not on crates.io).
- Server API used: `GET /api/query?search_term=...` (JSON, added on this
  branch) and the existing `POST /send-to-rqbit/` (form: `magnet`, `category`).

## Build and run

```sh
cd anorak-gpui
cargo build --release
./target/release/anorak-gpui --server http://192.168.0.101:9341
```

The server URL comes from `--server URL`, then `$ANORAK_SERVER`, and falls back
to `http://192.168.0.101:9341`. `--size 1280x800` sets the initial window size.

This crate is its own Cargo workspace (it is not a member of the server's).
That keeps GPUI's dependency tree (712 packages in `Cargo.lock`, about 450
built on Windows) out of the server's
`Cargo.lock`, which `nix/package.nix` uses to build the deployed server.

`rust-toolchain.toml` pins `stable`. On a Windows host whose rustup default
host is GNU, build with `cargo +stable-x86_64-pc-windows-msvc build --release`
because GPUI's DirectX backend expects MSVC. It needs the Windows SDK
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

See `docs/gpui-comparison.md` in the repo root for the results.
