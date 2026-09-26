# Local changes to Zed's gpui_wgpu (rev 933d8d93)

The crate is otherwise a verbatim copy of `crates/gpui_wgpu` at that rev.

1. **Lazy render pipelines on wasm** (`src/wgpu_renderer.rs`,
   `WgpuPipelines`). Upstream creates all nine pipelines when the renderer is
   built. On WebGL2 every pipeline is a GL program, and wgpu checks its link
   status right away (`getProgramParameter(LINK_STATUS)`), which blocks the
   main thread while ANGLE compiles the shaders. With a cold shader cache that
   was about 400 ms of the ~500 ms from wasm start to first frame in Chrome and
   Firefox on Windows. Pipelines are now built on first use on wasm, so the
   first frame compiles only what it draws (quads and monochrome text).
   Native builds still create everything up front.
2. **No blocking `device.poll(Wait)` in `update_drawable_size` on wasm.**
   It cannot block in a browser (wgpu's GLES backend waits with a zero
   timeout on WebGL, and polling is a no-op on WebGPU), so it only logged
   "Failed to poll device during resize: Timeout". Browsers keep destroyed
   textures alive until queued work completes.
