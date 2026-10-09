//! Colors: the app's own palette (src/app/app.css, src/terminal/theme.ts),
//! dark or light as macOS is, painted by x8ai so it looks the same in every
//! terminal (ADR 0025). Exact where the terminal shows 24-bit color, else the
//! nearest of its 256 colors.

use ratatui::style::Color;

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    truecolor: bool,
    dark: bool,
}

/// Whether this terminal shows 24-bit color: `COLORTERM`, which such
/// terminals set.
pub fn truecolor_here() -> bool {
    std::env::var("COLORTERM").is_ok_and(|v| v == "truecolor" || v == "24bit")
}

/// Whether macOS shows dark mode, as the app follows it.
pub fn dark_here() -> bool {
    std::process::Command::new("/usr/bin/defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .stderr(std::process::Stdio::null())
        .output()
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).trim() == "Dark")
}

/// The app's colors, as `0xRRGGBB`.
struct Palette {
    bg: u32,
    raised: u32,
    hover: u32,
    active: u32,
    border_strong: u32,
    text: u32,
    muted: u32,
    faint: u32,
    accent: u32,
    accent_fill: u32,
    highlight: u32,
    ok: u32,
    error: u32,
    selection: u32,
    /// The terminal's 16 colors: black, red, green, yellow, blue, magenta,
    /// cyan, white, then their bright forms.
    ansi: [u32; 16],
}

const DARK: Palette = Palette {
    bg: 0x141414,
    raised: 0x1b1b1b,
    hover: 0x242424,
    active: 0x2d2d2d,
    border_strong: 0x333333,
    text: 0xe8e8e8,
    muted: 0x9c9c9c,
    faint: 0x636363,
    accent: 0x7c6cff,
    accent_fill: 0x4326ff,
    highlight: 0xe0607e,
    ok: 0x3fb950,
    error: 0xff5d5d,
    selection: 0x3a3080,
    ansi: [
        0x202020, 0xff6b6b, 0x5fbf6a, 0xd9a441, 0x7c6cff, 0xe0607e, 0x4fc3cf, 0xbdbdbd, 0x6e6e6e,
        0xff8f8f, 0x7fd48a, 0xecc062, 0x9d91ff, 0xf08aa3, 0x72d9e3, 0xf2f2f2,
    ],
};

const LIGHT: Palette = Palette {
    bg: 0xf6f6f6,
    raised: 0xececec,
    hover: 0xe2e2e2,
    active: 0xd6d6d6,
    border_strong: 0xcdcdcd,
    text: 0x1a1a1a,
    muted: 0x5c5c5c,
    faint: 0x8c8c8c,
    accent: 0x3d1fff,
    accent_fill: 0x3d1fff,
    highlight: 0xc2415f,
    ok: 0x1a7f37,
    error: 0xcf222e,
    selection: 0xd4ceff,
    ansi: [
        0x1f1f1f, 0xcf222e, 0x116329, 0x9a6700, 0x3d1fff, 0xc2415f, 0x1b7c83, 0x6e6e6e, 0x575757,
        0xa40e26, 0x1a7f37, 0x633c01, 0x5a42ff, 0xd85a78, 0x3192aa, 0x8c8c8c,
    ],
};

impl Theme {
    /// As this process's terminal shows color, until one attaches.
    pub fn detect() -> Self {
        Self::new(truecolor_here(), true)
    }

    pub fn new(truecolor: bool, dark: bool) -> Self {
        Self { truecolor, dark }
    }

    fn palette(self) -> &'static Palette {
        if self.dark { &DARK } else { &LIGHT }
    }

    fn hex(self, value: u32) -> Color {
        let [_, r, g, b] = value.to_be_bytes();
        self.rgb(r, g, b)
    }

    pub fn rgb(self, r: u8, g: u8, b: u8) -> Color {
        if self.truecolor {
            Color::Rgb(r, g, b)
        } else {
            Color::Indexed(nearest_256(r, g, b))
        }
    }

    /// The screen behind everything, and a terminal's default background.
    pub fn bg(self) -> Color {
        self.hex(self.palette().bg)
    }

    /// Raised: bars, the sidebar, boxes.
    pub fn raised(self) -> Color {
        self.hex(self.palette().raised)
    }

    pub fn hover(self) -> Color {
        self.hex(self.palette().hover)
    }

    /// What is selected, and keys shown as keys.
    pub fn active(self) -> Color {
        self.hex(self.palette().active)
    }

    pub fn border_strong(self) -> Color {
        self.hex(self.palette().border_strong)
    }

    /// Text, and a terminal's default foreground.
    pub fn text(self) -> Color {
        self.hex(self.palette().text)
    }

    pub fn muted(self) -> Color {
        self.hex(self.palette().muted)
    }

    pub fn faint(self) -> Color {
        self.hex(self.palette().faint)
    }

    /// Indigo: names, what is focused.
    pub fn accent(self) -> Color {
        self.hex(self.palette().accent)
    }

    /// The stronger indigo, behind white text: the command line's bar, the
    /// button that agrees.
    pub fn accent_fill(self) -> Color {
        self.hex(self.palette().accent_fill)
    }

    /// White, on the accent fill.
    pub fn on_fill(self) -> Color {
        self.rgb(0xff, 0xff, 0xff)
    }

    /// Raspberry: the greeting's SIR, the caret, what needs attention.
    pub fn highlight(self) -> Color {
        self.hex(self.palette().highlight)
    }

    pub fn ok(self) -> Color {
        self.hex(self.palette().ok)
    }

    pub fn error(self) -> Color {
        self.hex(self.palette().error)
    }

    /// Behind text selected in a terminal.
    pub fn selection(self) -> Color {
        self.hex(self.palette().selection)
    }

    /// A terminal's color `index` (0–15), as the app's terminals show it.
    pub fn ansi(self, index: u8) -> Color {
        self.hex(self.palette().ansi[usize::from(index % 16)])
    }

    /// The terminal's own background and cursor set to the app's, for the
    /// margins around the grid; `client.rs` puts them back.
    pub fn terminal_colors(self) -> String {
        let [_, r, g, b] = self.palette().bg.to_be_bytes();
        let [_, cr, cg, cb] = self.palette().highlight.to_be_bytes();
        format!("\x1b]11;#{r:02x}{g:02x}{b:02x}\x07\x1b]12;#{cr:02x}{cg:02x}{cb:02x}\x07")
    }
}

/// The xterm-256 color closest to `r g b`: from the 6×6×6 cube or the grey ramp.
pub fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let level = |v: u8| {
        (0..6)
            .min_by_key(|&i| LEVELS[i].abs_diff(v))
            .unwrap_or_default()
    };
    let (ri, gi, bi) = (level(r), level(g), level(b));
    let cube = (LEVELS[ri], LEVELS[gi], LEVELS[bi]);
    let cube_index = 16 + 36 * ri + 6 * gi + bi;

    // The grey ramp: 24 steps from 8 to 238.
    let average = (u16::from(r) + u16::from(g) + u16::from(b)) / 3;
    let step = usize::from(average.saturating_sub(3) / 10).min(23);
    let grey = 8 + 10 * step as u8;
    let grey_index = 232 + step;

    let distance = |(cr, cg, cb): (u8, u8, u8)| {
        let d = |a: u8, b: u8| u32::from(a.abs_diff(b)).pow(2);
        d(cr, r) + d(cg, g) + d(cb, b)
    };
    let index = if distance((grey, grey, grey)) < distance(cube) {
        grey_index
    } else {
        cube_index
    };
    index as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palette_is_the_apps() {
        let dark = Theme::new(true, true);
        assert_eq!(dark.bg(), Color::Rgb(0x14, 0x14, 0x14));
        assert_eq!(dark.highlight(), Color::Rgb(0xe0, 0x60, 0x7e));
        assert_eq!(dark.ansi(1), Color::Rgb(0xff, 0x6b, 0x6b));
        let light = Theme::new(true, false);
        assert_eq!(light.bg(), Color::Rgb(0xf6, 0xf6, 0xf6));
        assert_eq!(light.text(), Color::Rgb(0x1a, 0x1a, 0x1a));
        // Without 24-bit color: the nearest of 256, a near-black grey.
        assert_eq!(Theme::new(false, true).bg(), Color::Indexed(233));
        assert!(dark.terminal_colors().contains("]11;#141414"));
    }

    #[test]
    fn colors_map_to_the_nearest_of_256() {
        assert_eq!(nearest_256(0, 0, 0), 16);
        assert_eq!(nearest_256(255, 255, 255), 231);
        assert_eq!(nearest_256(255, 0, 0), 196);
        // Raspberry: a pinkish red in the cube.
        assert_eq!(nearest_256(0xe0, 0x60, 0x7e), 168);
        // A mid grey goes to the grey ramp, not the cube.
        assert_eq!(nearest_256(0x80, 0x80, 0x80), 244);
    }
}
