//! The atomic unit of the terminal grid: one character cell, plus the color
//! and attribute types it carries. Pure data — no behavior, no allocation.

use bitflags::bitflags;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A foreground or background color. `Default` defers to the renderer's
/// theme; `Palette` indexes the 256-color table; `Rgb` is 24-bit truecolor.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Color {
    #[default]
    Default,
    Palette(u8),
    Rgb([u8; 3]),
}

bitflags! {
    /// The eight SGR rendition flags. Packed into a u8 so a `Cell` stays small.
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
    pub struct Attrs: u8 {
        const BOLD          = 0b0000_0001;
        const DIM           = 0b0000_0010;
        const ITALIC        = 0b0000_0100;
        const UNDERLINE     = 0b0000_1000;
        const BLINK         = 0b0001_0000;
        const INVERSE       = 0b0010_0000;
        const INVISIBLE     = 0b0100_0000;
        const STRIKETHROUGH = 0b1000_0000;
    }
}

// Manual serde for Attrs: serialize/deserialize as a transparent u8.
impl Serialize for Attrs {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u8(self.bits())
    }
}

impl<'de> Deserialize<'de> for Attrs {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let bits = u8::deserialize(d)?;
        Ok(Attrs::from_bits_truncate(bits))
    }
}

/// One grid cell: a Unicode scalar plus its rendition. A freshly-default
/// `Cell` is a blank space with the theme colors and no attributes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Cell {
    /// The Unicode codepoint stored in this cell.
    pub cp: char,
    pub fg: Color,
    pub bg: Color,
    pub attrs: Attrs,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            cp: ' ',
            fg: Color::Default,
            bg: Color::Default,
            attrs: Attrs::empty(),
        }
    }
}

impl Cell {
    /// True when this cell is visually identical to a default blank — used by
    /// scrollback row trimming to drop trailing empty cells.
    pub fn is_blank(&self) -> bool {
        *self == Cell::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cell_is_a_blank_space_with_theme_colors() {
        let cell = Cell::default();
        assert_eq!(cell.cp, ' ');
        assert_eq!(cell.fg, Color::Default);
        assert_eq!(cell.bg, Color::Default);
        assert!(cell.is_blank());
    }

    #[test]
    fn is_blank_distinguishes_content_and_rendition() {
        let cell = Cell {
            cp: 'x',
            ..Cell::default()
        };
        assert!(!cell.is_blank());

        let colored = Cell {
            bg: Color::Palette(4),
            ..Cell::default()
        };
        assert!(!colored.is_blank());

        let attributed = Cell {
            attrs: Attrs::BOLD,
            ..Cell::default()
        };
        assert!(!attributed.is_blank());
    }

    #[test]
    fn color_variants_compare_by_value() {
        assert_eq!(Color::Palette(7), Color::Palette(7));
        assert_ne!(Color::Palette(7), Color::Palette(8));
        assert_eq!(Color::Rgb([1, 2, 3]), Color::Rgb([1, 2, 3]));
        assert_ne!(Color::Default, Color::Rgb([0, 0, 0]));
    }

    #[test]
    fn attrs_defaults_are_all_false() {
        let attrs = Attrs::default();
        assert!(!attrs.contains(Attrs::BOLD));
        assert!(!attrs.contains(Attrs::DIM));
        assert!(!attrs.contains(Attrs::ITALIC));
        assert!(!attrs.contains(Attrs::UNDERLINE));
        assert!(!attrs.contains(Attrs::BLINK));
        assert!(!attrs.contains(Attrs::INVERSE));
        assert!(!attrs.contains(Attrs::INVISIBLE));
        assert!(!attrs.contains(Attrs::STRIKETHROUGH));
    }
}
