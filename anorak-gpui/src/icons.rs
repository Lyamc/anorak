//! The few monochrome icons the UI draws (Sort panel). They are glyphs of a
//! tiny embedded font ("Anorak Icons", ~1.4 KB) built from the SVG sources in
//! assets/icons by assets/icons/build-font.py, so they render through GPUI's
//! text path: tinted by the text colour, crisp at any scale, and no SVG
//! rasterizer (resvg) in the wasm bundle. Drawn for this project.

use std::borrow::Cow;

use gpui::App;

pub const FAMILY: &str = "Anorak Icons";
const FONT_DATA: &[u8] = include_bytes!("../assets/fonts/anorak-icons/AnorakIcons.ttf");

pub const FONT: &str = "\u{E000}";
pub const WEIGHT: &str = "\u{E001}";
pub const SEEDLING: &str = "\u{E002}";
pub const CLOCK: &str = "\u{E003}";
pub const ARROW_UP: &str = "\u{E004}";
pub const ARROW_DOWN: &str = "\u{E005}";
pub const XMARK: &str = "\u{E006}";
pub const PLUS: &str = "\u{E007}";

/// Register the icon font with GPUI's text system (both targets).
pub fn register(cx: &mut App) {
    if let Err(err) = cx.text_system().add_fonts(vec![Cow::Borrowed(FONT_DATA)]) {
        log::error!("failed to load icon font: {err:#}");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn font_is_truetype() {
        assert_eq!(&super::FONT_DATA[..4], &[0, 1, 0, 0]);
    }
}
