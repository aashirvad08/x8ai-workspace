//! Colors. The app's raspberry and indigo where the terminal shows 24-bit
//! color, and the nearest of its 256 colors where it does not (Terminal.app
//! before macOS 26). Text and backgrounds stay the terminal's own, so the
//! user's light or dark theme is kept; muted text is dimmed, not grey.

use ratatui::style::Color;

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    truecolor: bool,
}

/// Whether this terminal shows 24-bit color: `COLORTERM`, which such
/// terminals set.
pub fn truecolor_here() -> bool {
    std::env::var("COLORTERM").is_ok_and(|v| v == "truecolor" || v == "24bit")
}

impl Theme {
    /// As this process's terminal shows color, until one attaches.
    pub fn detect() -> Self {
        Self::new(truecolor_here())
    }

    pub fn new(truecolor: bool) -> Self {
        Self { truecolor }
    }

    pub fn rgb(self, r: u8, g: u8, b: u8) -> Color {
        if self.truecolor {
            Color::Rgb(r, g, b)
        } else {
            Color::Indexed(nearest_256(r, g, b))
        }
    }

    /// Raspberry: the greeting's SIR, the caret, what needs attention.
    pub fn highlight(self) -> Color {
        self.rgb(0xe0, 0x60, 0x7e)
    }

    /// Indigo: the selected suggestion, the space's name.
    pub fn accent(self) -> Color {
        self.rgb(0x7c, 0x6c, 0xff)
    }

    pub fn ok(self) -> Color {
        self.rgb(0x3f, 0xb9, 0x50)
    }

    pub fn error(self) -> Color {
        self.rgb(0xff, 0x5d, 0x5d)
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
