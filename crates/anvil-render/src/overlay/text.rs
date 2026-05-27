//! Text rendering seam for overlay widgets.
//!
//! `OverlayPainter` abstracts the glyph painter so widgets are font-agnostic.
//! `MonoPainter` wraps the current monospace `GlyphPainter`; a future
//! `UiPainter` will swap in a proportional font for non-code labels (Track A).

use crate::raster::{FontMetrics, GlyphPainter, Raster};

/// Font weight hint. `MonoPainter` ignores this (mono has one weight today);
/// `UiPainter` will select the appropriate variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    Regular,
    Bold,
}

/// Abstract text painter used by all overlay widgets.
pub trait OverlayPainter {
    /// Draw a label string at the given pixel position.
    ///
    /// `x`, `y` are the top-left of the first glyph cell in device pixels.
    /// `color` is `[R, G, B]`.
    fn label(&mut self, x: f64, y: f64, s: &str, color: [u8; 3], weight: Weight);

    /// Measure the pixel width of a string (number of chars × cell_w for mono).
    fn measure(&self, s: &str, weight: Weight) -> f64;

    /// Cell height (for row layout).
    fn cell_h(&self) -> f64;

    /// Cell width (for column layout).
    fn cell_w(&self) -> f64;
}

/// Monospace implementation wrapping the existing `GlyphPainter`.
///
/// All chars are assumed to occupy exactly `metrics.cell_w` each.
pub struct MonoPainter<'a> {
    pub raster: &'a mut Raster,
    pub painter: &'a mut dyn GlyphPainter,
    pub metrics: FontMetrics,
}

impl<'a> MonoPainter<'a> {
    pub fn new(
        raster: &'a mut Raster,
        painter: &'a mut dyn GlyphPainter,
        metrics: FontMetrics,
    ) -> Self {
        Self {
            raster,
            painter,
            metrics,
        }
    }
}

impl OverlayPainter for MonoPainter<'_> {
    fn label(&mut self, x: f64, y: f64, s: &str, color: [u8; 3], _weight: Weight) {
        let cw = self.metrics.cell_w;
        let mut cx = x;
        for ch in s.chars() {
            self.raster
                .glyph_at(self.painter, self.metrics, cx, y, ch as u32, color);
            cx += cw;
        }
    }

    fn measure(&self, s: &str, _weight: Weight) -> f64 {
        s.chars().count() as f64 * self.metrics.cell_w
    }

    fn cell_h(&self) -> f64 {
        self.metrics.cell_h
    }

    fn cell_w(&self) -> f64 {
        self.metrics.cell_w
    }
}
