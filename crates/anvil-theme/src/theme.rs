//! Terminal color themes. A [`Theme`] is plain data; [`resolve`] produces an
//! active theme from a base name plus optional per-color overrides.
//!
//! The Mineral palette and semantic status colors are a brand contract —
//! values must match BRAND.md exactly.

use serde::{Deserialize, Serialize};

use crate::color::hex_to_rgb;

// ── WCAG contrast helpers ────────────────────────────────────────────────────

/// WCAG 2.x contrast ratio between two sRGB colors.
/// Returns a value in [1, 21]; 21 is black-on-white.
pub fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f64 {
    let la = relative_luminance(a);
    let lb = relative_luminance(b);
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

fn linearize(c: u8) -> f64 {
    let s = c as f64 / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055_f64).powf(2.4)
    }
}

fn relative_luminance(rgb: [u8; 3]) -> f64 {
    0.2126 * linearize(rgb[0]) + 0.7152 * linearize(rgb[1]) + 0.0722 * linearize(rgb[2])
}

// ── Theme ────────────────────────────────────────────────────────────────────

/// A complete terminal color theme.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Theme {
    /// Primary canvas background.
    pub background: [u8; 3],
    /// Primary text / foreground.
    pub foreground: [u8; 3],
    /// Cursor / primary accent color.
    pub accent: [u8; 3],
    /// Raised panel / card surfaces (HUD, tree, cheatsheet, active tab).
    pub surface: [u8; 3],
    /// Panel edges / separators.
    pub border: [u8; 3],
    /// The 16 ANSI palette entries.
    pub ansi: [[u8; 3]; 16],
}

impl Theme {
    /// xterm-style 256-color lookup.
    ///
    /// - Slots 0–15 come from `ansi`.
    /// - Slots 16–231 are the 6×6×6 RGB cube.
    /// - Slots 232–255 are the grayscale ramp.
    pub fn palette256(&self, index: u8) -> [u8; 3] {
        if index < 16 {
            return self.ansi[index as usize];
        }
        if index < 232 {
            let i = (index as usize) - 16;
            const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
            return [LEVELS[(i / 36) % 6], LEVELS[(i / 6) % 6], LEVELS[i % 6]];
        }
        let v = (8u16 + 10 * (index as u16 - 232)) as u8;
        [v, v, v]
    }
}

// ── Built-in palettes ────────────────────────────────────────────────────────

/// Mineral Dark — a soft, slightly-pastel palette on a calm (not pure-black)
/// canvas. Brand hue families are kept: mineral teal, semantic red/green/amber,
/// agent violet, steel blue. Normal ANSI colors are lifted to luminous pastels
/// so they read on the dark canvas; `bright-black` (ansi[8]) is a visible dim
/// blue-grey at ≥5:1 contrast. ansi[0] (ANSI black) is just enough above the
/// background to be distinguishable (≥1.3:1). border is nudged to ≥1.4:1
/// against surface.
pub const MINERAL_DARK: Theme = Theme {
    background: [0x18, 0x1a, 0x21],
    foreground: [0xd8, 0xdb, 0xe2],
    accent: [0x54, 0xb7, 0xc0],  // luminous mineral
    surface: [0x22, 0x26, 0x2f], // #22262f — clear lift above canvas
    border: [0x3a, 0x40, 0x4e],  // #3a404e — panel edges (1.46:1 vs surface)
    ansi: [
        [0x2c, 0x30, 0x3e],
        [0xe0, 0x8b, 0x82],
        [0x8e, 0xc9, 0x9b],
        [0xe2, 0xc0, 0x89],
        [0x8b, 0xb0, 0xd4],
        [0xbb, 0xa6, 0xdd],
        [0x7e, 0xca, 0xce],
        [0xc3, 0xc8, 0xd2],
        [0x86, 0x8e, 0xa6],
        [0xee, 0x9f, 0x96],
        [0xa6, 0xd8, 0xb1],
        [0xef, 0xce, 0x9a],
        [0xa3, 0xc4, 0xe4],
        [0xcb, 0xb8, 0xe9],
        [0x95, 0xd9, 0xde],
        [0xee, 0xf1, 0xf6],
    ],
};

/// Mineral Light — a refined reader-mode palette on the brand bone canvas.
/// ANSI colors are mid-deep and gently muted; every slot is ≥4.5:1 on bone.
/// Fixes vs prior version: accent darkened (#2c7a82 → #286e76, was 4.39:1),
/// ansi[6] cyan matches accent, ansi[7] dark-grey replaces too-pale #7a828b
/// (was 3.43:1), ansi[9] bright-red darkened (was 4.21:1), ansi[15] bright-
/// white replaced: original #f6f8f9 was 1.07:1 on bone — now a mid steel-grey.
pub const MINERAL_LIGHT: Theme = Theme {
    background: [0xee, 0xf1, 0xf2], // bone
    foreground: [0x1b, 0x1f, 0x24], // ink
    accent: [0x28, 0x6e, 0x76],     // mineral teal — 5.16:1 on bone
    surface: [0xff, 0xff, 0xff],    // #ffffff — raised light panels (BRAND.md: "white only")
    border: [0xd4, 0xd9, 0xdc],     // #d4d9dc — panel edges on bone (1.42:1 vs white)
    ansi: [
        [0x1b, 0x1f, 0x24],
        [0xb5, 0x44, 0x3a],
        [0x32, 0x79, 0x52],
        [0x94, 0x64, 0x10],
        [0x3f, 0x6c, 0x95],
        [0x62, 0x55, 0x8f],
        [0x28, 0x6e, 0x76],
        [0x5e, 0x65, 0x6d],
        [0x5d, 0x66, 0x71],
        [0xad, 0x40, 0x33],
        [0x35, 0x78, 0x50],
        [0x86, 0x59, 0x0e],
        [0x37, 0x60, 0x8a],
        [0x56, 0x4a, 0x83],
        [0x25, 0x6a, 0x70],
        [0x5f, 0x67, 0x6f],
    ],
};

// ── Overrides ────────────────────────────────────────────────────────────────

/// ANSI-slot overrides — each field is an optional `#rrggbb` hex string.
/// Absent fields keep the base theme value.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnsiOverrides {
    pub black: Option<String>,
    pub red: Option<String>,
    pub green: Option<String>,
    pub yellow: Option<String>,
    pub blue: Option<String>,
    pub magenta: Option<String>,
    pub cyan: Option<String>,
    pub white: Option<String>,
    pub bright_black: Option<String>,
    pub bright_red: Option<String>,
    pub bright_green: Option<String>,
    pub bright_yellow: Option<String>,
    pub bright_blue: Option<String>,
    pub bright_magenta: Option<String>,
    pub bright_cyan: Option<String>,
    pub bright_white: Option<String>,
}

/// Per-color overrides applied on top of the chosen base theme.
/// Every field is optional; an absent field keeps the base theme value.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeOverrides {
    pub background: Option<String>,
    pub foreground: Option<String>,
    pub accent: Option<String>,
    #[serde(default)]
    pub ansi: AnsiOverrides,
}

// ── Resolution ───────────────────────────────────────────────────────────────

/// Resolve a base theme by name. An unknown name falls back to [`MINERAL_DARK`]
/// and prints a warning to stderr.
pub fn by_name(name: &str) -> Theme {
    match name {
        "mineral-dark" => MINERAL_DARK,
        "mineral-light" => MINERAL_LIGHT,
        other => {
            eprintln!("anvil: unknown theme \"{other}\", using mineral-dark");
            MINERAL_DARK
        }
    }
}

/// Apply one optional override hex string to an RGB slot. A bad hex string is
/// logged to stderr and leaves `slot` unchanged.
fn apply_override(slot: &mut [u8; 3], maybe_hex: Option<&str>) {
    let Some(hex) = maybe_hex else { return };
    match hex_to_rgb(hex) {
        Ok(rgb) => *slot = rgb,
        Err(_) => eprintln!("anvil: invalid theme color \"{hex}\", ignored"),
    }
}

/// Build the active theme: base theme `name` with `overrides` applied on top.
pub fn resolve(name: &str, overrides: &ThemeOverrides) -> Theme {
    let mut t = by_name(name);
    apply_override(&mut t.background, overrides.background.as_deref());
    apply_override(&mut t.foreground, overrides.foreground.as_deref());
    apply_override(&mut t.accent, overrides.accent.as_deref());
    apply_override(&mut t.ansi[0], overrides.ansi.black.as_deref());
    apply_override(&mut t.ansi[1], overrides.ansi.red.as_deref());
    apply_override(&mut t.ansi[2], overrides.ansi.green.as_deref());
    apply_override(&mut t.ansi[3], overrides.ansi.yellow.as_deref());
    apply_override(&mut t.ansi[4], overrides.ansi.blue.as_deref());
    apply_override(&mut t.ansi[5], overrides.ansi.magenta.as_deref());
    apply_override(&mut t.ansi[6], overrides.ansi.cyan.as_deref());
    apply_override(&mut t.ansi[7], overrides.ansi.white.as_deref());
    apply_override(&mut t.ansi[8], overrides.ansi.bright_black.as_deref());
    apply_override(&mut t.ansi[9], overrides.ansi.bright_red.as_deref());
    apply_override(&mut t.ansi[10], overrides.ansi.bright_green.as_deref());
    apply_override(&mut t.ansi[11], overrides.ansi.bright_yellow.as_deref());
    apply_override(&mut t.ansi[12], overrides.ansi.bright_blue.as_deref());
    apply_override(&mut t.ansi[13], overrides.ansi.bright_magenta.as_deref());
    apply_override(&mut t.ansi[14], overrides.ansi.bright_cyan.as_deref());
    apply_override(&mut t.ansi[15], overrides.ansi.bright_white.as_deref());
    t
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // --- by_name ---

    #[test]
    fn by_name_resolves_built_in_themes() {
        assert_eq!(by_name("mineral-dark").background, MINERAL_DARK.background);
        assert_eq!(
            by_name("mineral-light").background,
            MINERAL_LIGHT.background
        );
    }

    #[test]
    fn by_name_falls_back_to_dark_for_unknown() {
        assert_eq!(by_name("nope").background, MINERAL_DARK.background);
    }

    // --- palette256 ---

    #[test]
    fn palette256_covers_three_ranges() {
        // ANSI slot (0-15)
        assert_eq!(MINERAL_DARK.palette256(6), [0x7e, 0xca, 0xce]);
        // 6x6x6 cube: index 16 → i=0 → [0,0,0]
        assert_eq!(MINERAL_DARK.palette256(16), [0, 0, 0]);
        // 6x6x6 cube: index 231 → i=215 → [255,255,255]
        assert_eq!(MINERAL_DARK.palette256(231), [255, 255, 255]);
        // Grayscale: index 232 → v = 8+10*0 = 8
        assert_eq!(MINERAL_DARK.palette256(232), [8, 8, 8]);
        // Grayscale: index 255 → v = 8+10*23 = 238
        assert_eq!(MINERAL_DARK.palette256(255), [238, 238, 238]);
    }

    // --- resolve ---

    #[test]
    fn resolve_with_no_overrides_equals_base() {
        let t = resolve("mineral-dark", &ThemeOverrides::default());
        assert_eq!(t.background, MINERAL_DARK.background);
        assert_eq!(t.ansi[2], MINERAL_DARK.ansi[2]);
    }

    #[test]
    fn resolve_applies_valid_override() {
        let ov = ThemeOverrides {
            background: Some("#101316".into()),
            ansi: AnsiOverrides {
                green: Some("#52b070".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let t = resolve("mineral-dark", &ov);
        assert_eq!(t.background, [0x10, 0x13, 0x16]);
        assert_eq!(t.ansi[2], [0x52, 0xb0, 0x70]);
        assert_eq!(t.foreground, MINERAL_DARK.foreground); // untouched
    }

    #[test]
    fn resolve_keeps_base_for_invalid_hex_override() {
        let ov = ThemeOverrides {
            accent: Some("not-a-color".into()),
            ..Default::default()
        };
        let t = resolve("mineral-dark", &ov);
        assert_eq!(t.accent, MINERAL_DARK.accent);
    }

    // --- contrast_ratio ---

    #[test]
    fn contrast_ratio_black_vs_white_is_21() {
        let r = contrast_ratio([0, 0, 0], [255, 255, 255]);
        assert!((r - 21.0).abs() < 0.01);
    }

    #[test]
    fn contrast_ratio_identical_colors_is_1() {
        let r = contrast_ratio([0x18, 0x1a, 0x21], [0x18, 0x1a, 0x21]);
        assert!((r - 1.0).abs() < 0.001);
    }

    // --- WCAG brand-quality audit ---

    #[test]
    fn mineral_dark_wcag_contrast_targets() {
        let bg = MINERAL_DARK.background;
        let surface = MINERAL_DARK.surface;
        let border = MINERAL_DARK.border;

        // foreground >= 9:1
        assert!(contrast_ratio(MINERAL_DARK.foreground, bg) >= 9.0);
        // accent >= 4.5:1
        assert!(contrast_ratio(MINERAL_DARK.accent, bg) >= 4.5);
        // border >= 1.4:1 vs surface
        assert!(contrast_ratio(border, surface) >= 1.4);
        // ansi[0] (black) >= 1.3:1 vs background (distinguishable cell)
        assert!(contrast_ratio(MINERAL_DARK.ansi[0], bg) >= 1.3);
        // ansi[1..15] (text colors) >= 4.5:1 vs background
        for color in &MINERAL_DARK.ansi[1..] {
            assert!(
                contrast_ratio(*color, bg) >= 4.5,
                "ansi color {color:?} fails 4.5:1 on dark background"
            );
        }
    }

    #[test]
    fn mineral_light_wcag_contrast_targets() {
        let bg = MINERAL_LIGHT.background;
        let surface = MINERAL_LIGHT.surface;
        let border = MINERAL_LIGHT.border;

        // foreground >= 9:1
        assert!(contrast_ratio(MINERAL_LIGHT.foreground, bg) >= 9.0);
        // accent >= 4.5:1
        assert!(contrast_ratio(MINERAL_LIGHT.accent, bg) >= 4.5);
        // border >= 1.4:1 vs surface
        assert!(contrast_ratio(border, surface) >= 1.4);
        // ansi[0] (black) >= 1.3:1 vs background
        assert!(contrast_ratio(MINERAL_LIGHT.ansi[0], bg) >= 1.3);
        // ansi[1..15] (text colors) >= 4.5:1 vs background
        for color in &MINERAL_LIGHT.ansi[1..] {
            assert!(
                contrast_ratio(*color, bg) >= 4.5,
                "ansi color {color:?} fails 4.5:1 on light background"
            );
        }
    }
}
