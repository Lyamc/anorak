//! Light and dark palettes, following the system appearance (the OS app
//! mode on native, `prefers-color-scheme` in the browser; GPUI reports both
//! through `Window::appearance`). Dark is the web UI's dark palette (the CSS
//! custom properties in assets/style.css); light is the web UI's light one.

use std::cell::Cell;

use gpui::{Hsla, WindowAppearance, rgba};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub dark: bool,
    // --bg, --text, --muted
    pub bg: u32,
    pub text: u32,
    pub muted: u32,
    // --surface, --surface-hover
    pub surface: u32,
    pub surface_hover: u32,
    // --border (search box, dropdown toggles), --border-strong, --panel-border
    pub border: u32,
    pub border_strong: u32,
    pub panel_border: u32,
    // --accent-bg (table header, open popovers)
    pub accent_bg: u32,
    // --primary-bg / -text / -hover (Grab selected, active sort field)
    pub primary_bg: u32,
    pub primary_text: u32,
    pub primary_hover: u32,
    // --row-hover, --row-selected
    pub row_hover: u32,
    pub row_selected: u32,
    /// Failed search / grab text (the web UI shows no errors).
    pub error: u32,
    /// Checked checkbox fill and its tick (Chrome's light / dark checkbox).
    pub check_bg: u32,
    pub check_fg: u32,
    // Text input: placeholder, caret, selection (RGBA).
    pub placeholder: u32,
    pub caret: u32,
    pub selection: u32,
}

pub const LIGHT: Theme = Theme {
    dark: false,
    bg: 0xDDDBDE,
    text: 0x3B373B,
    muted: 0x888888,
    surface: 0xFFFFFF,
    surface_hover: 0xFCFCFC,
    border: 0xCAD4DF,
    border_strong: 0x656E77,
    panel_border: 0xB8C2CD,
    accent_bg: 0xCAD4DF,
    primary_bg: 0x3B373B,
    primary_text: 0xFFFFFF,
    primary_hover: 0x555555,
    row_hover: 0xFCFCFC,
    row_selected: 0xEEF1F5,
    error: 0xB00020,
    check_bg: 0x0B57D0,
    check_fg: 0xFFFFFF,
    placeholder: 0x00000033,
    caret: 0x0066FFFF,
    selection: 0x3311FF30,
};

pub const DARK: Theme = Theme {
    dark: true,
    bg: 0x1B1C1F,
    text: 0xE4E2E5,
    muted: 0xA3A3A8,
    surface: 0x26282C,
    surface_hover: 0x2F3237,
    border: 0x3D434B,
    border_strong: 0x8A95A1,
    panel_border: 0x4A5767,
    accent_bg: 0x33404D,
    primary_bg: 0xD7DCE2,
    primary_text: 0x1B1C1F,
    primary_hover: 0xBCC4CD,
    row_hover: 0x2A2D32,
    row_selected: 0x2B3644,
    error: 0xF28B82,
    check_bg: 0x99C8FF,
    check_fg: 0x1B1C1F,
    placeholder: 0x9AA0A6FF,
    caret: 0x8AB8F0FF,
    selection: 0x8AB8F055,
};

impl Theme {
    pub fn for_appearance(appearance: WindowAppearance) -> &'static Theme {
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => &DARK,
            WindowAppearance::Light | WindowAppearance::VibrantLight => &LIGHT,
        }
    }

    pub fn name(&self) -> &'static str {
        if self.dark { "dark" } else { "light" }
    }
}

thread_local! {
    static CURRENT: Cell<&'static Theme> = const { Cell::new(&LIGHT) };
}

/// The palette in use (GPUI renders on one thread).
pub fn get() -> &'static Theme {
    CURRENT.with(|c| c.get())
}

/// Switch to the palette for `appearance`; returns whether it changed.
pub fn set_appearance(appearance: WindowAppearance) -> bool {
    let next = Theme::for_appearance(appearance);
    CURRENT.with(|c| {
        let changed = !std::ptr::eq(c.get(), next);
        c.set(next);
        changed
    })
}

/// `0xRRGGBBAA` as a colour.
pub fn ca(hex: u32) -> Hsla {
    rgba(hex).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_appearance() {
        assert!(!set_appearance(WindowAppearance::Light));
        assert!(!get().dark);
        assert!(set_appearance(WindowAppearance::Dark));
        assert!(get().dark);
        assert!(!set_appearance(WindowAppearance::VibrantDark));
        assert!(set_appearance(WindowAppearance::VibrantLight));
        assert_eq!(get(), &LIGHT);
    }

    #[test]
    fn dark_matches_web_css() {
        // assets/style.css, @media (prefers-color-scheme: dark)
        assert_eq!((DARK.bg, DARK.text, DARK.surface), (0x1B1C1F, 0xE4E2E5, 0x26282C));
        assert_eq!((DARK.accent_bg, DARK.panel_border, DARK.border_strong), (0x33404D, 0x4A5767, 0x8A95A1));
        assert_eq!((DARK.primary_bg, DARK.primary_text), (0xD7DCE2, 0x1B1C1F));
    }
}
