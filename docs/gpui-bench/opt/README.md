# Size and startup optimization: raw results (2026-09-26)

See docs/gpui-comparison.md, "Size and startup optimization". 3 runs per file.

- `{base,final}_webgl.jsonl`, `{base,final}_webgpu.jsonl`: wasmbench.mjs (Chrome 154), before (e06f127, uncompressed) / after (fa62dd9, precompressed).
- `{s,z}_webgl.jsonl`: opt-level s vs z of the combined build, Chrome WebGL2.
- `{base,final}_ff_{webgl,webgpu}.jsonl`: ffbench.mjs (Firefox 147).
- `{base,final}_mem.txt`: memprobe.mjs, wasm linear memory.
- `cjkprobe.txt`: cjkprobe.mjs, first-use cost of Canvas measureText per script in a fresh Chrome profile.
- `sizes_combo.txt`, `opt_variants_sizes.txt`, `wasm-opt-levels.txt`: bundle sizes, compression, bytes on the wire (wire.sh).
- `startprof-*.cpuprofile`: CDP CPU profiles of Chrome WebGL2 startup on the old build (startprof.mjs); open in Chrome DevTools > Performance.
- `bold-cyrillic-fallback.png`: expanded (bold) row with a Russian title on the subset SemiBold font (cyr.sh fixture + cyrcheck.mjs).
- `s_wasm2.sh`: deploy script (precompresses the dist, restarts the branch server and the fixture mocks).
