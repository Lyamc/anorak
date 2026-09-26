//! WebAssembly entry point: GPUI's single-threaded web platform on the page's
//! canvas, the bundled UI font, and settings from the page URL.

use std::borrow::Cow;
use std::rc::Rc;
use std::sync::Arc;

use gpui::App;
use gpui_web::{CanvasFontFallback, WebBackendPreference, WebPlatform};
use wasm_bindgen::prelude::*;
use web_time::Instant;

use crate::bench::Bench;

/// The browser has no system fonts for GPUI (it rasterizes with cosmic-text),
/// so the UI font ships inside the wasm. IBM Plex Sans is also what gpui_web
/// uses as its default family name. SIL OFL 1.1, see assets/fonts.
pub const UI_FONT: &str = "IBM Plex Sans";
const FONTS: [&[u8]; 2] = [
    include_bytes!("../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf"),
    include_bytes!("../assets/fonts/ibm-plex-sans/IBMPlexSans-SemiBold.ttf"),
];

fn location() -> web_sys::Location {
    web_sys::window().expect("window").location()
}

/// Value of `?name=...` in the page URL (percent-decoded), if present.
pub fn query_param(name: &str) -> Option<String> {
    let search = location().search().ok()?;
    search.trim_start_matches('?').split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (k == name).then(|| {
            js_sys::decode_uri_component(&v.replace('+', " "))
                .map(String::from)
                .unwrap_or_else(|_| v.to_string())
        })
    })
}

fn backend_preference() -> WebBackendPreference {
    match query_param("backend").as_deref() {
        Some("webgpu") => WebBackendPreference::WebGpu,
        Some("webgl") | Some("webgl2") => WebBackendPreference::WebGl,
        _ => WebBackendPreference::Auto,
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    let main_start = Instant::now();
    console_error_panic_hook::set_once();
    gpui_web::init_logging();

    // Same-origin by default: the Anorak server serves this bundle under /gpui/.
    let server = query_param("server")
        .unwrap_or_else(|| location().origin().unwrap_or_default())
        .trim_end_matches('/')
        .to_string();
    let flag = |name: &str| query_param(name).is_some_and(|v| v != "0");
    let bench = (flag("bench") || flag("selftest")).then(|| Bench {
        term: query_param("term").unwrap_or_else(|| "one punch".to_string()),
        main_start,
        selftest: flag("selftest"),
    });

    // Titles can contain CJK (the fixture has a few); the bundled Latin font
    // lacks it, so let gpui_web draw those graphemes with the browser's fonts.
    let platform = Rc::new(WebPlatform::new_with_backend_and_font_fallback(
        false,
        backend_preference(),
        CanvasFontFallback::EmojiAndCjk,
    ));
    let http = Arc::new(platform.fetch_http_client());
    gpui::Application::with_platform(platform)
        .with_http_client(http)
        .run(move |cx: &mut App| {
            let fonts = FONTS.iter().map(|b| Cow::Borrowed(*b)).collect();
            if let Err(err) = cx.text_system().add_fonts(fonts) {
                web_sys::console::error_1(&format!("failed to load UI fonts: {err:#}").into());
            }
            crate::launch(cx, server, bench, None);
        });
}
