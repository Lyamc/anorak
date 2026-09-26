//! anorak-gpui: front-end for the Anorak Torznab search server, built on GPUI
//! (Zed's GPU-accelerated UI framework). One code base, two targets:
//!
//! * native desktop (Windows/macOS/Linux) via `gpui_platform`;
//! * the browser, compiled to `wasm32-unknown-unknown` and run on GPUI's web
//!   platform (`gpui_web`: one canvas, WebGPU with WebGL2 fallback).
//!
//! Native usage: anorak-gpui [--server URL] [--size WxH] [--bench LOGFILE] [--bench-term TERM]
//! The server URL can also come from ANORAK_SERVER; the default is
//! http://192.168.0.101:9341. The server must expose `GET /api/query`.
//!
//! Web: the Anorak server serves the Trunk bundle under `/gpui/` and the app
//! talks to the page's own origin. Query parameters: `backend=webgpu|webgl`
//! (default: auto), `server=URL`, `bench=1`, `selftest=1`, `term=TEXT`.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![cfg_attr(target_family = "wasm", no_main)]

mod api;
mod app;
mod bench;
mod model;
mod text_input;

#[cfg(target_family = "wasm")]
mod web;

use gpui::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};

#[cfg(not(target_family = "wasm"))]
const DEFAULT_SERVER: &str = "http://192.168.0.101:9341";

#[cfg(not(target_family = "wasm"))]
struct Args {
    server: String,
    bench: Option<std::path::PathBuf>,
    bench_term: String,
    size: (f32, f32),
    selftest: bool,
}

#[cfg(not(target_family = "wasm"))]
fn parse_args() -> Args {
    use std::path::PathBuf;
    let mut args = Args {
        server: std::env::var("ANORAK_SERVER").unwrap_or_else(|_| DEFAULT_SERVER.to_string()),
        bench: std::env::var_os("ANORAK_GPUI_BENCH").map(PathBuf::from),
        bench_term: "one punch".to_string(),
        size: (1280., 800.),
        selftest: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--server" | "-s" => {
                if let Some(v) = it.next() {
                    args.server = v;
                }
            }
            "--bench" => args.bench = it.next().map(PathBuf::from),
            "--selftest" => {
                args.bench = it.next().map(PathBuf::from);
                args.selftest = true;
            }
            "--bench-term" => {
                if let Some(v) = it.next() {
                    args.bench_term = v;
                }
            }
            "--size" => {
                if let Some((w, h)) = it.next().as_deref().and_then(|v| v.split_once('x')) {
                    if let (Ok(w), Ok(h)) = (w.parse(), h.parse()) {
                        args.size = (w, h);
                    }
                }
            }
            "-h" | "--help" => {
                println!(
                    "anorak-gpui [--server URL] [--size WxH] [--bench LOGFILE] [--bench-term TERM]\n\
                     Server defaults to $ANORAK_SERVER or {DEFAULT_SERVER}."
                );
                std::process::exit(0);
            }
            other if other.starts_with("http://") || other.starts_with("https://") => {
                args.server = other.to_string()
            }
            other => eprintln!("ignoring unknown argument {other:?}"),
        }
    }
    args.server = args.server.trim_end_matches('/').to_string();
    args
}

#[cfg(not(target_family = "wasm"))]
fn main() {
    let main_start = web_time::Instant::now();
    let args = parse_args();
    let bench = args.bench.map(|out| bench::Bench {
        out,
        term: args.bench_term.clone(),
        main_start,
        selftest: args.selftest,
    });
    let (win_w, win_h) = args.size;
    gpui_platform::application().run(move |cx: &mut App| {
        launch(cx, args.server, bench, Some((win_w, win_h)));
    });
}

/// Shared by both targets once the platform is up: key bindings and the one window.
fn launch(cx: &mut App, server: String, bench: Option<bench::Bench>, win: Option<(f32, f32)>) {
    text_input::bind_keys(cx);
    app::bind_keys(cx);
    let title = format!("Anorak Search — {server}");
    let window_bounds = win.map(|(w, h)| {
        WindowBounds::Windowed(Bounds::centered(None, size(px(w), px(h)), cx))
    });
    cx.open_window(
        WindowOptions {
            window_bounds,
            titlebar: Some(TitlebarOptions {
                title: Some(title.into()),
                ..Default::default()
            }),
            window_min_size: win.map(|_| size(px(720.), px(400.))),
            ..Default::default()
        },
        move |window, cx| cx.new(|cx| app::AnorakApp::new(server, bench, window, cx)),
    )
    .expect("failed to open window");
    cx.activate(true);
}
