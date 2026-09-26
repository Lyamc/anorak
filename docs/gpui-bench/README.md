# Measurement harness for docs/gpui-comparison.md

- `mock.py FIXTURE.xml POSTLOG`: mock Torznab server on 127.0.0.1:18420 that
  always returns the fixture, plus a mock rqbit on 127.0.0.1:18030 that
  records POST /torrents and adds nothing. Start the server under test with
  `JACKETT_URL=http://127.0.0.1:18420/api/v2.0/indexers/all/results/torznab
  JACKETT_APIKEY=fixture RQBIT_URL=http://127.0.0.1:18030 ANORAK_PORT=...`.
- `webbench.mjs URL RUNS`: run it with Node and `playwright-core` on the
  client machine. Chrome runs headed with a fresh profile per run. Set the
  idle-CPU window with `IDLE_S`.
- `gpuibench.ps1 -Runs 5 -IdleS 20 -Server URL`: launches
  `anorak-gpui --bench` and samples its memory and CPU with `Get-Process`.
- `results-web.json` and `results-gpui.json`: the raw 5-run results behind
  the tables (`results-gpui.json` has a UTF-8 BOM from PowerShell).

WebAssembly client (`feature/gpui-wasm`, see the last section of
`docs/gpui-comparison.md`):

- `s_wasm.sh PORT`: on the server host, starts the branch server with
  `ANORAK_GPUI_DIST` pointing at the trunk `dist/`, plus `mock.py`, then
  prints the headers of each bundle file.
- `wasmbench.mjs URL RUNS` (Chrome, playwright-core): URL such as
  `http://HOST:PORT/gpui/?bench=1&backend=webgl`. `CHROME_ARGS` adds flags,
  e.g. `--unsafely-treat-insecure-origin-as-secure=http://HOST:PORT` for
  WebGPU. `OUT_JSONL` appends each finished run to a file.
- `ffbench.mjs URL RUNS` (Firefox, puppeteer-core over WebDriver BiDi):
  `MODE=bench` or `MODE=parity`. `FF_PREFS='{"dom.securecontext.allowlist":"HOST"}'`
  enables WebGPU on plain http.
- `parity.mjs URL OUTDIR`, with `STEPS=layout,typing,ime,clipboard,mouse_select,tab,tooltip,popovers,grab,resize,wheel,wheelfine,info`:
  functional checks with CDP input and screenshots. `DSF`/`NOVIEWPORT` and
  `CHROME_ARGS=--force-device-scale-factor=2` are for HiDPI.
  `cliptest.mjs` is the clipboard-only check.
- `sizes.sh`: raw/gzip/brotli/zstd sizes of the bundle (needs nix-shell).
  The output is in `sizes-wasm.txt`.
- Raw results: `results-gpui-zedgit.json` (native, Zed git GPUI, 5 runs),
  `results-wasm-chrome-{webgl,webgpu}.json` (5 runs each),
  `results-wasm-firefox-webgl.json` (5 runs) and
  `results-wasm-firefox-webgpu.json` (4 runs; the 5th was interrupted and
  discarded).
