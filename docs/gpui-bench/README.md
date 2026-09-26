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
