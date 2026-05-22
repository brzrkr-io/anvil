//! Low-level color primitives: RGB, ClearColor, hex parsing, and color mixing.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A normalized, fully-opaque RGBA color in linear [0, 1] space.
/// Matches the Metal clear-color layout used by the Zig renderer.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct ClearColor {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

/// Error returned by hex parsing functions.
#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorError {
    #[error("invalid hex color (expected #rrggbb or rrggbb)")]
    InvalidHex,
}

/// Linearly blend two RGB colors. `t=0` → `a`, `t=1` → `b`.
/// Channel values are rounded to the nearest integer.
pub fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let c = t.clamp(0.0, 1.0);
    let mut out = [0u8; 3];
    for i in 0..3 {
        let av = a[i] as f32;
        let bv = b[i] as f32;
        out[i] = (av + (bv - av) * c).round() as u8;
    }
    out
}

/// Parse a `#rrggbb` (or bare `rrggbb`) hex string into a normalized,
/// fully-opaque [`ClearColor`]. Returns [`ColorError::InvalidHex`] on bad
/// length or non-hex digits.
pub fn hex_to_clear_color(hex: &str) -> Result<ClearColor, ColorError> {
    let s = hex.strip_prefix('#').unwrap_or(hex);
    if s.len() != 6 {
        return Err(ColorError::InvalidHex);
    }
    let r = u8::from_str_radix(&s[0..2], 16).map_err(|_| ColorError::InvalidHex)?;
    let g = u8::from_str_radix(&s[2..4], 16).map_err(|_| ColorError::InvalidHex)?;
    let b = u8::from_str_radix(&s[4..6], 16).map_err(|_| ColorError::InvalidHex)?;
    Ok(ClearColor {
        r: r as f64 / 255.0,
        g: g as f64 / 255.0,
        b: b as f64 / 255.0,
        a: 1.0,
    })
}

/// Parse a `#rrggbb` (or bare `rrggbb`) hex string into raw RGB bytes.
pub fn hex_to_rgb(hex: &str) -> Result<[u8; 3], ColorError> {
    let s = hex.strip_prefix('#').unwrap_or(hex);
    if s.len() != 6 {
        return Err(ColorError::InvalidHex);
    }
    let r = u8::from_str_radix(&s[0..2], 16).map_err(|_| ColorError::InvalidHex)?;
    let g = u8::from_str_radix(&s[2..4], 16).map_err(|_| ColorError::InvalidHex)?;
    let b = u8::from_str_radix(&s[4..6], 16).map_err(|_| ColorError::InvalidHex)?;
    Ok([r, g, b])
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- hex_to_clear_color ---

    #[test]
    fn parses_hash_rrggbb() {
        let c = hex_to_clear_color("#0b0d0e").unwrap();
        let eps = 1e-9;
        assert!((c.r - 0x0b as f64 / 255.0).abs() < eps);
        assert!((c.g - 0x0d as f64 / 255.0).abs() < eps);
        assert!((c.b - 0x0e as f64 / 255.0).abs() < eps);
        assert_eq!(c.a, 1.0);
    }

    #[test]
    fn accepts_hex_without_hash() {
        let c = hex_to_clear_color("0b0d0e").unwrap();
        let eps = 1e-9;
        assert!((c.r - 0x0b as f64 / 255.0).abs() < eps);
    }

    #[test]
    fn rejects_wrong_length() {
        assert_eq!(hex_to_clear_color("#fff"), Err(ColorError::InvalidHex));
        assert_eq!(hex_to_clear_color(""), Err(ColorError::InvalidHex));
    }

    #[test]
    fn rejects_non_hex_digits() {
        assert_eq!(hex_to_clear_color("#zzzzzz"), Err(ColorError::InvalidHex));
    }

    // --- mix ---

    #[test]
    fn mix_at_t0_returns_a() {
        let a = [10u8, 20, 30];
        let b = [200u8, 150, 100];
        assert_eq!(mix(a, b, 0.0), a);
    }

    #[test]
    fn mix_at_t1_returns_b() {
        let a = [10u8, 20, 30];
        let b = [200u8, 150, 100];
        assert_eq!(mix(a, b, 1.0), b);
    }

    #[test]
    fn mix_at_half_returns_midpoint() {
        let a = [0u8, 0, 0];
        let b = [200u8, 100, 50];
        let m = mix(a, b, 0.5);
        assert!((m[0] as i16 - 100).abs() <= 1);
        assert!((m[1] as i16 - 50).abs() <= 1);
        assert!((m[2] as i16 - 25).abs() <= 1);
    }
}
