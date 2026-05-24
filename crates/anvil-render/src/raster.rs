//! Pure-Rust BGRA8 pixel-buffer rasterizer.
//!
//! `Raster` owns the bitmap and performs all rectangle/pixel work itself,
//! with no platform dependency.  Glyph drawing is delegated to an external
//! `GlyphPainter` so that `anvil-render` stays pure and `anvil-platform`
//! can supply a CoreText implementation later.
//!
//! # Coordinate conventions
//!
//! `Raster` uses a **top-down** bitmap coordinate system (row 0 at the top).
//! This matches Metal texture uploads, unlike the CG context (which is y-up)
//! in the original Zig implementation.  The arithmetic in `cell_rect_top` and
//! `row_rule_y` replicates the same layout as `raster.zig` but expressed in
//! top-down bitmap-y rather than CG y-up coordinates.

/// Font metrics needed by the rasterizer.  Passed by value so `Raster` has
/// no lifetime dependency on a font object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontMetrics {
    /// Width of one monospace cell in device pixels.
    pub cell_w: f64,
    /// Height of one monospace cell in device pixels.
    pub cell_h: f64,
    /// Baseline descent (positive number of pixels below baseline) — used by
    /// `GlyphPainter` to position glyphs within the cell.
    pub descent: f64,
}

/// A rectangle in device pixels (top-down bitmap space).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PixelRect {
    /// Left edge (device pixels from left of bitmap).
    pub x: f64,
    /// Top edge (device pixels from top of bitmap).
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Trait implemented by callers that can draw a single glyph onto the pixel
/// buffer.  `anvil-platform` will implement this with CoreText; tests supply
/// a stub.
///
/// # Contract
/// The implementor receives the cell's Unicode codepoint, the destination
/// rectangle in top-down bitmap pixels, the foreground RGB color, and the
/// font metrics. It must resolve the codepoint to a glyph through the font
/// cmap, then write BGRA8 pixels into the provided buffer slice, which covers
/// exactly `stride * height_pixels` bytes where `stride = full_bitmap_width * 4`.
pub trait GlyphPainter {
    /// Draw the glyph for Unicode `codepoint` into the pixel buffer. The
    /// implementor resolves the codepoint to a font glyph index itself.
    ///
    /// `dest` is the cell rectangle in top-down bitmap coordinates.
    /// `fg` is the foreground color as `[R, G, B]`.
    /// `metrics` provides cell dimensions and descent.
    /// `pixels` is the full BGRA8 bitmap buffer (width × height × 4 bytes).
    /// `bitmap_width` and `bitmap_height` are the full buffer dimensions.
    #[allow(clippy::too_many_arguments)]
    fn draw_glyph(
        &mut self,
        codepoint: u32,
        dest: PixelRect,
        fg: [u8; 3],
        metrics: FontMetrics,
        pixels: &mut [u8],
        bitmap_width: usize,
        bitmap_height: usize,
    );
}

/// A BGRA8 pixel-buffer rasterizer.
///
/// All rectangle operations are pure Rust.  Glyph drawing is routed through
/// `&mut dyn GlyphPainter` so the crate stays platform-free.
pub struct Raster {
    /// BGRA8 pixel buffer, length == `width * height * 4`.
    pixels: Vec<u8>,
    pub width: usize,
    pub height: usize,
    /// Horizontal padding in device pixels (applied by `cell_rect`).
    pub pad_x: f64,
    /// Vertical padding in device pixels (applied by `cell_rect`).
    pub pad_y: f64,
    /// Vertical pixel offset added to every cell (smooth-scroll animation).
    pub y_shift_px: f64,
    /// Device-pixel x of cell-column 0 for the current pane.
    pub origin_x: f64,
    /// Device-pixel y (top-down) of cell-row 0 for the current pane.
    pub origin_y: f64,
}

impl Raster {
    /// Allocate a new raster at the given dimensions (minimum 1×1).
    pub fn new(width: usize, height: usize) -> Self {
        let w = width.max(1);
        let h = height.max(1);
        Raster {
            pixels: vec![0u8; w * h * 4],
            width: w,
            height: h,
            pad_x: 0.0,
            pad_y: 0.0,
            y_shift_px: 0.0,
            origin_x: 0.0,
            origin_y: 0.0,
        }
    }

    /// Resize the pixel buffer.  A no-op when dimensions are unchanged.
    /// The buffer is zeroed when resized.
    pub fn resize(&mut self, width: usize, height: usize) {
        let w = width.max(1);
        let h = height.max(1);
        if w == self.width && h == self.height {
            return;
        }
        self.pixels = vec![0u8; w * h * 4];
        self.width = w;
        self.height = h;
    }

    /// Fill the whole bitmap with one color.
    pub fn clear(&mut self, rgb: [u8; 3]) {
        let [r, g, b] = rgb;
        for px in self.pixels.chunks_exact_mut(4) {
            px[0] = b; // B
            px[1] = g; // G
            px[2] = r; // R
            px[3] = 0xff; // A
        }
    }

    /// Fill a horizontal pixel-row band with one color.
    ///
    /// `y_top` and `y_bottom` are device-pixel Y coordinates (top-down).
    /// Rows outside `[0, height)` are silently clamped.
    pub fn clear_pixel_rows(&mut self, y_top: usize, y_bottom: usize, rgb: [u8; 3]) {
        let [r, g, b] = rgb;
        let y0 = y_top.min(self.height);
        let y1 = y_bottom.min(self.height);
        if y0 >= y1 {
            return;
        }
        let stride = self.width * 4;
        let start = y0 * stride;
        let end = y1 * stride;
        for px in self.pixels[start..end].chunks_exact_mut(4) {
            px[0] = b;
            px[1] = g;
            px[2] = r;
            px[3] = 0xff;
        }
    }

    /// Fill one cell's background at integer cell coordinates.
    pub fn cell_bg(&mut self, metrics: FontMetrics, col: usize, row: usize, rgb: [u8; 3]) {
        let rect = self.cell_rect(metrics, col as f64, row as f64);
        self.fill_pixel_rect_internal(rect, rgb);
    }

    /// Fill a sub-rectangle of a cell.  Used for bar/underline cursors.
    ///
    /// `col` and `row` are fractional cell coordinates.
    /// `fx`,`fy` are offsets from the cell's **top-left** corner as fractions
    /// of cell dimensions.  `fw`,`fh` are width/height fractions.
    ///
    /// Note: the Zig original uses a CG y-up context so `fy=0` is the cell
    /// **bottom**.  Here we stay in top-down bitmap space.  Callers that
    /// previously passed `fy=0` for "bottom strip" now pass `fy=(1-fh)` — but
    /// the draw.rs module handles this translation to keep the public contract
    /// consistent with the Zig original.
    #[allow(clippy::too_many_arguments)]
    pub fn cell_inset(
        &mut self,
        metrics: FontMetrics,
        col: f64,
        row: f64,
        rgb: [u8; 3],
        fx: f64,
        fy: f64,
        fw: f64,
        fh: f64,
    ) {
        let r = self.cell_rect(metrics, col, row);
        let rect = PixelRect {
            x: r.x + r.w * fx,
            y: r.y + r.h * fy,
            w: r.w * fw,
            h: r.h * fh,
        };
        self.fill_pixel_rect_internal(rect, rgb);
    }

    /// Draw one glyph in a cell.  `glyph_id == 0` (missing glyph) is a no-op.
    /// The call is routed to `painter`, which owns the CoreText logic.
    pub fn cell_glyph(
        &mut self,
        painter: &mut dyn GlyphPainter,
        metrics: FontMetrics,
        col: usize,
        row: usize,
        glyph_id: u32,
        rgb: [u8; 3],
    ) {
        if glyph_id == 0 {
            return;
        }
        let dest = self.cell_rect(metrics, col as f64, row as f64);
        let w = self.width;
        let h = self.height;
        painter.draw_glyph(glyph_id, dest, rgb, metrics, &mut self.pixels, w, h);
    }

    /// Draw a small filled square in the left gutter (the `pad_x` band) on the
    /// given viewport `row`, vertically centered on that cell row.
    ///
    /// The square is 6×6 device pixels, placed 2px inside the left padding band:
    ///   x = origin_x - pad_x + 2
    ///   y = row midpoint - 3  (centered)
    ///
    /// This positions the marker inside the visual margin, not on cell content.
    pub fn gutter_mark(&mut self, metrics: FontMetrics, row: usize, rgb: [u8; 3]) {
        const MARK_SIZE: f64 = 6.0;
        const MARK_INSET: f64 = 2.0;
        let ch = metrics.cell_h;
        // Horizontal: inside the pad_x band, 2px from the content edge.
        let x = self.origin_x - self.pad_x + MARK_INSET;
        // Vertical: center of the row in top-down space (respects y_shift_px).
        let row_top = self.origin_y + row as f64 * ch - self.y_shift_px;
        let y = row_top + (ch - MARK_SIZE) * 0.5;
        self.fill_pixel_rect_internal(
            PixelRect {
                x,
                y,
                w: MARK_SIZE,
                h: MARK_SIZE,
            },
            rgb,
        );
    }

    /// Draw a 2 px wide full-row-height accent bar in the left padding band.
    ///
    /// Used for block-based command output: a colored vertical stripe runs the
    /// full height of the row along the far-left edge of `pad_x`, indicating
    /// block membership and status.
    pub fn block_accent_bar(&mut self, metrics: FontMetrics, row: usize, rgb: [u8; 3]) {
        // Full cell-wide stripe at the LEFT edge of the cell grid. Reads as a
        // clear vertical band so the eye groups the rows of one block.
        let cw = metrics.cell_w;
        let ch = metrics.cell_h;
        let x = self.origin_x;
        let row_top = self.origin_y + row as f64 * ch - self.y_shift_px;
        self.fill_pixel_rect_internal(
            PixelRect {
                x,
                y: row_top,
                w: cw,
                h: ch,
            },
            rgb,
        );
    }

    /// Draw a thin full-height vertical hairline at the LEFT edge of cell-column
    /// `col`.  The strip is 2 device pixels wide.
    pub fn col_rule(&mut self, metrics: FontMetrics, col: usize, rgb: [u8; 3]) {
        let cw = metrics.cell_w;
        let left_x = self.origin_x + col as f64 * cw;
        let rect = PixelRect {
            x: left_x,
            y: 0.0,
            w: 2.0,
            h: self.height as f64,
        };
        self.fill_pixel_rect_internal(rect, rgb);
    }

    /// Draw a thin horizontal hairline at the TOP edge of cell-row `row`.
    /// The strip is 2 device pixels tall and respects `y_shift_px` so it
    /// scrolls with the grid.
    ///
    /// `x_start` and `x_end` are device-pixel bounds so the rule can start at
    /// the terminal content's left edge.
    pub fn row_rule(
        &mut self,
        metrics: FontMetrics,
        row: f64,
        rgb: [u8; 3],
        x_start: f64,
        x_end: f64,
    ) {
        let ch = metrics.cell_h;
        // In top-down bitmap space: the top of row N is at bitmap-y = origin_y + N*ch - y_shift_px.
        // 1px hairline — matches the tab-bar bottom rule and cheatsheet edge.
        let strip_h = 1.0;
        let top_y = self.origin_y + row * ch - self.y_shift_px;
        let rect = PixelRect {
            x: x_start,
            y: top_y,
            w: x_end - x_start,
            h: strip_h,
        };
        self.fill_pixel_rect_internal(rect, rgb);
    }

    /// Fill an arbitrary rectangle.  `px`, `py` are the top-left corner in
    /// top-down device-pixel coordinates.
    pub fn fill_pixel_rect(&mut self, px: f64, py: f64, pw: f64, ph: f64, rgb: [u8; 3]) {
        self.fill_pixel_rect_internal(
            PixelRect {
                x: px,
                y: py,
                w: pw,
                h: ph,
            },
            rgb,
        );
    }

    /// Like `fill_pixel_rect` but composites at `alpha` over existing content.
    pub fn fill_pixel_rect_alpha(
        &mut self,
        px: f64,
        py: f64,
        pw: f64,
        ph: f64,
        rgb: [u8; 3],
        alpha: f64,
    ) {
        let rect = PixelRect {
            x: px,
            y: py,
            w: pw,
            h: ph,
        };
        let (x0, y0, x1, y1) = self.clip_rect(rect);
        let [r, g, b] = rgb;
        let a = alpha.clamp(0.0, 1.0) as f32;
        let inv = 1.0 - a;
        for y in y0..y1 {
            for x in x0..x1 {
                let i = (y * self.width + x) * 4;
                let ob = self.pixels[i];
                let og = self.pixels[i + 1];
                let or_ = self.pixels[i + 2];
                self.pixels[i] = (b as f32 * a + ob as f32 * inv).round() as u8;
                self.pixels[i + 1] = (g as f32 * a + og as f32 * inv).round() as u8;
                self.pixels[i + 2] = (r as f32 * a + or_ as f32 * inv).round() as u8;
                self.pixels[i + 3] = 0xff;
            }
        }
    }

    /// The BGRA8 pixel buffer, ready for texture upload.
    pub fn bytes(&self) -> &[u8] {
        &self.pixels
    }

    // ── internals ────────────────────────────────────────────────────────────

    /// Compute the cell rectangle for fractional cell coordinates in top-down
    /// bitmap space.
    pub fn cell_rect(&self, metrics: FontMetrics, col: f64, row: f64) -> PixelRect {
        let cw = metrics.cell_w;
        let ch = metrics.cell_h;
        PixelRect {
            x: self.origin_x + col * cw,
            y: self.origin_y + row * ch - self.y_shift_px,
            w: cw,
            h: ch,
        }
    }

    fn fill_pixel_rect_internal(&mut self, rect: PixelRect, rgb: [u8; 3]) {
        let (x0, y0, x1, y1) = self.clip_rect(rect);
        let [r, g, b] = rgb;
        for y in y0..y1 {
            let row_start = (y * self.width + x0) * 4;
            let row_end = (y * self.width + x1) * 4;
            let row = &mut self.pixels[row_start..row_end];
            for px in row.chunks_exact_mut(4) {
                px[0] = b;
                px[1] = g;
                px[2] = r;
                px[3] = 0xff;
            }
        }
    }

    /// Clip a floating-point rectangle to valid pixel integer bounds.
    fn clip_rect(&self, rect: PixelRect) -> (usize, usize, usize, usize) {
        let x0 = (rect.x.max(0.0) as usize).min(self.width);
        let y0 = (rect.y.max(0.0) as usize).min(self.height);
        let x1 = ((rect.x + rect.w).max(0.0) as usize).min(self.width);
        let y1 = ((rect.y + rect.h).max(0.0) as usize).min(self.height);
        (x0, y0, x1, y1)
    }
}

/// Read the RGB triple at pixel `(x, y)` from a raster (for tests).
pub fn pixel_at(r: &Raster, x: usize, y: usize) -> [u8; 3] {
    let i = (y * r.width + x) * 4;
    [r.pixels[i + 2], r.pixels[i + 1], r.pixels[i]] // BGRA → RGB
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // A no-op GlyphPainter that records calls instead of painting pixels.
    #[derive(Default)]
    struct StubPainter {
        pub calls: Vec<(u32, [u8; 3])>, // (glyph_id, fg)
    }

    impl GlyphPainter for StubPainter {
        #[allow(clippy::too_many_arguments)]
        fn draw_glyph(
            &mut self,
            glyph_id: u32,
            _dest: PixelRect,
            fg: [u8; 3],
            _metrics: FontMetrics,
            _pixels: &mut [u8],
            _bitmap_width: usize,
            _bitmap_height: usize,
        ) {
            self.calls.push((glyph_id, fg));
        }
    }

    fn default_metrics() -> FontMetrics {
        FontMetrics {
            cell_w: 10.0,
            cell_h: 20.0,
            descent: 4.0,
        }
    }

    // ── Ported from raster.zig ────────────────────────────────────────────

    /// Port of "clear fills the bitmap"
    #[test]
    fn clear_fills_bitmap() {
        let mut r = Raster::new(64, 48);
        r.clear([10, 20, 30]);
        assert_eq!(pixel_at(&r, 5, 5), [10, 20, 30]);
        assert_eq!(pixel_at(&r, 60, 40), [10, 20, 30]);
    }

    /// Port of "cellBg and glyph draw onto the bitmap" — bg portion only;
    /// glyph pixels are tested via stub call recording.
    #[test]
    fn cell_bg_paints_cell_region() {
        let m = default_metrics(); // 10×20 cells
        let mut r = Raster::new(400, 200);
        r.clear([0, 0, 0]);
        r.cell_bg(m, 1, 1, [80, 0, 0]);
        // A pixel at the center of cell (1,1): x ≈ 15, y ≈ 30 (origin 0,0).
        let cx = (m.cell_w * 1.5) as usize;
        let cy = (m.cell_h * 1.5) as usize;
        assert_eq!(pixel_at(&r, cx, cy), [80, 0, 0]);
    }

    /// Port of glyph draw side of "cellBg and glyph draw onto the bitmap" —
    /// uses stub painter, asserts call is recorded.
    #[test]
    fn cell_glyph_routes_to_painter() {
        let m = default_metrics();
        let mut r = Raster::new(400, 200);
        r.clear([0, 0, 0]);
        let mut painter = StubPainter::default();
        // glyph_id 42, fg white
        r.cell_glyph(&mut painter, m, 1, 1, 42, [255, 255, 255]);
        assert_eq!(painter.calls.len(), 1);
        assert_eq!(painter.calls[0], (42, [255, 255, 255]));
    }

    /// Port of "glyph_id 0 is a no-op"
    #[test]
    fn cell_glyph_zero_id_is_noop() {
        let m = default_metrics();
        let mut r = Raster::new(100, 100);
        let mut painter = StubPainter::default();
        r.cell_glyph(&mut painter, m, 0, 0, 0, [255, 0, 0]);
        assert!(painter.calls.is_empty());
    }

    /// Port of "resize keeps a usable context"
    #[test]
    fn resize_keeps_usable_buffer() {
        let mut r = Raster::new(32, 32);
        r.resize(128, 64);
        assert_eq!(r.width, 128);
        r.clear([7, 7, 7]);
        assert_eq!(pixel_at(&r, 100, 50), [7, 7, 7]);
    }

    /// Port of "cellInset fills a sub-rectangle of a cell" (left bar)
    #[test]
    fn cell_inset_left_bar() {
        let m = default_metrics(); // 10×20 cells
        let mut r = Raster::new(400, 200);
        r.clear([0, 0, 0]);
        // Left bar: 20% width, full height, of cell (2,1).
        // In top-down space: fx=0, fy=0 is top-left.
        // 20% of 10px = 2px → covers x=[20,22); probe at x=21 is safely inside.
        r.cell_inset(m, 2.0, 1.0, [90, 0, 0], 0.0, 0.0, 0.20, 1.0);
        let lx = (m.cell_w * 2.0 + 1.0) as usize; // == 21, inside [20,22)
        let ly = (m.cell_h * 1.5) as usize;
        assert_eq!(pixel_at(&r, lx, ly), [90, 0, 0]);
        // Right half stays clear.
        let rx = (m.cell_w * 2.8) as usize;
        assert_eq!(pixel_at(&r, rx, ly), [0, 0, 0]);
    }

    /// Port of "cellInset underline fills the cell bottom"
    ///
    /// The Zig version used CG y-up (fy=0 at cell bottom).  In top-down space
    /// the bottom strip has fy = (1 - fh), which draw.rs handles.  Here we
    /// test the raw `cell_inset` with top-down `fy`.
    #[test]
    fn cell_inset_underline_bottom_strip() {
        let m = default_metrics(); // 10×20 cells
        let mut r = Raster::new(400, 200);
        r.clear([0, 0, 0]);
        let fh = 0.12_f64;
        // Bottom strip in top-down: fy = 1 - fh
        r.cell_inset(m, 2.0, 1.0, [0, 0, 200], 0.0, 1.0 - fh, 1.0, fh);
        let ux = (m.cell_w * 2.5) as usize;
        // The cell (2,1) occupies bitmap rows [cell_h, 2*cell_h) = [20, 40).
        // Bottom strip is the last fh fraction: starts at 40 - 20*0.12 = 37.6 → 37.
        let bot_y = (m.cell_h * 2.0 - 2.0) as usize;
        assert_eq!(pixel_at(&r, ux, bot_y), [0, 0, 200]);
        // Upper half should remain clear.
        let mid_y = (m.cell_h * 1.5) as usize;
        assert_eq!(pixel_at(&r, ux, mid_y), [0, 0, 0]);
    }

    /// Port of "rowRule draws a strip at the top of a cell row"
    #[test]
    fn row_rule_draws_at_top_of_row() {
        let m = default_metrics(); // 10×20 cells
        let w = 400usize;
        let h = 300usize;
        let mut r = Raster::new(w, h);
        r.clear([0, 0, 0]);
        r.row_rule(m, 1.0, [200, 100, 50], 0.0, w as f64);
        // In top-down space: the top of row 1 is at bitmap-y = 0 + 1.0 * 20 - 0 = 20.
        let strip_bitmap_y = m.cell_h as usize; // == 20
        let mid_x = w / 2;
        assert_eq!(pixel_at(&r, mid_x, strip_bitmap_y), [200, 100, 50]);
        // Interior of row 1 (well below top edge) stays clear.
        let inner_y = (m.cell_h * 1.5) as usize;
        assert_eq!(pixel_at(&r, mid_x, inner_y), [0, 0, 0]);
    }

    // ── Stale-separator tests (Bug E regression) ─────────────────────────

    /// Port of "clear then draw nothing leaves a pure background bitmap"
    #[test]
    fn clear_then_no_draw_gives_pure_background() {
        let mut r = Raster::new(64, 48);
        let bg = [20u8, 20, 20];
        r.clear(bg);
        assert_eq!(pixel_at(&r, 0, 0), bg);
        assert_eq!(pixel_at(&r, 32, 24), bg);
        assert_eq!(pixel_at(&r, 63, 47), bg);
    }

    /// Port of "rowRule draws only on its row and clear erases it next frame"
    #[test]
    fn row_rule_only_on_row_and_clear_erases_it() {
        let m = default_metrics(); // 10×20 cells
        let w = 400usize;
        let h = 300usize;
        let mut r = Raster::new(w, h);
        let bg = [0u8, 0, 0];
        let rule_color = [200u8, 100, 50];

        // Frame 1: draw a rule at row 2.
        r.clear(bg);
        r.row_rule(m, 2.0, rule_color, 0.0, w as f64);
        let strip_y = (m.cell_h * 2.0) as usize; // bitmap-y = 40
        assert_eq!(pixel_at(&r, w / 2, strip_y), rule_color);
        // Row 1 interior stays bg.
        let row1_mid = (m.cell_h * 1.5) as usize;
        assert_eq!(pixel_at(&r, w / 2, row1_mid), bg);

        // Frame 2: clear only — rule must be gone.
        r.clear(bg);
        assert_eq!(pixel_at(&r, w / 2, strip_y), bg);
    }

    /// Port of "y_shift_px shifts cellBg upward in the bitmap"
    #[test]
    fn y_shift_moves_cell_bg() {
        let m = default_metrics(); // 10×20 cells
        let w = 400usize;
        let h = 200usize;

        let mut r_base = Raster::new(w, h);
        r_base.clear([0, 0, 0]);
        r_base.cell_bg(m, 1, 1, [255, 0, 0]);
        let cx = (m.cell_w * 1.5) as usize;
        // Near the bottom of cell (1,1): bitmap-y ≈ 1.9 * cell_h = 38.
        let cy = (m.cell_h * 1.9) as usize;
        let base_px = pixel_at(&r_base, cx, cy);

        // With a full-cell y_shift the cell shifts UP by cell_h in bitmap space
        // (y decreases in top-down coords): cell (1,1) now occupies [0, cell_h).
        let mut r_shift = Raster::new(w, h);
        r_shift.clear([0, 0, 0]);
        r_shift.y_shift_px = m.cell_h; // shift up
        r_shift.cell_bg(m, 1, 1, [255, 0, 0]);
        r_shift.y_shift_px = 0.0;
        let shift_px = pixel_at(&r_shift, cx, cy);

        // Without shift the row is inside the cell.
        assert_eq!(base_px, [255, 0, 0]);
        // With a full-cell upward shift that row is outside (clear).
        assert_eq!(shift_px, [0, 0, 0]);
    }

    // ── col_rule ─────────────────────────────────────────────────────────────

    #[test]
    fn col_rule_draws_vertical_strip_at_column() {
        let m = default_metrics(); // 10×20 cells
        let w = 400usize;
        let h = 200usize;
        let mut r = Raster::new(w, h);
        r.clear([0, 0, 0]);
        // Draw a col rule at col 3: starts at x=30, width=2px.
        r.col_rule(m, 3, [0, 200, 0]);
        // A pixel inside [30, 32) should be painted.
        assert_eq!(pixel_at(&r, 30, 50), [0, 200, 0]);
        assert_eq!(pixel_at(&r, 31, 50), [0, 200, 0]);
        // Pixel just left of the rule should remain clear.
        assert_eq!(pixel_at(&r, 29, 50), [0, 0, 0]);
    }

    // ── fill_pixel_rect_alpha ────────────────────────────────────────────────

    #[test]
    fn fill_pixel_rect_alpha_blends_over_background() {
        let mut r = Raster::new(100, 100);
        r.clear([0, 0, 0]); // black background
        // Paint a 10×10 rect at (10, 10) with 100% alpha red.
        r.fill_pixel_rect_alpha(10.0, 10.0, 10.0, 10.0, [255, 0, 0], 1.0);
        // At full alpha the result should be the foreground color.
        assert_eq!(pixel_at(&r, 15, 15), [255, 0, 0]);
    }

    #[test]
    fn fill_pixel_rect_alpha_zero_is_noop() {
        let mut r = Raster::new(100, 100);
        r.clear([50, 50, 50]);
        // 0 alpha: original background should remain.
        r.fill_pixel_rect_alpha(0.0, 0.0, 50.0, 50.0, [255, 0, 0], 0.0);
        assert_eq!(pixel_at(&r, 25, 25), [50, 50, 50]);
    }

    #[test]
    fn fill_pixel_rect_alpha_half_blends() {
        let mut r = Raster::new(100, 100);
        r.clear([0, 0, 0]);
        // 50% alpha white over black → ~[127, 127, 127] (rounded).
        r.fill_pixel_rect_alpha(0.0, 0.0, 10.0, 10.0, [255, 255, 255], 0.5);
        let px = pixel_at(&r, 5, 5);
        // Each channel should be ~128 (rounding may give 127 or 128).
        assert!(px[0] >= 127 && px[0] <= 128, "R channel: {}", px[0]);
        assert!(px[1] >= 127 && px[1] <= 128, "G channel: {}", px[1]);
        assert!(px[2] >= 127 && px[2] <= 128, "B channel: {}", px[2]);
    }

    // ── cell_rect ─────────────────────────────────────────────────────────────

    #[test]
    fn cell_rect_maps_fractional_cell_to_pixel_rect() {
        let m = default_metrics(); // 10×20
        let r = Raster::new(400, 200);
        let rect = r.cell_rect(m, 2.0, 3.0);
        // origin_x=0, origin_y=0, y_shift_px=0 by default.
        assert!((rect.x - 20.0).abs() < 1e-9); // 2 * 10
        assert!((rect.y - 60.0).abs() < 1e-9); // 3 * 20
        assert!((rect.w - 10.0).abs() < 1e-9);
        assert!((rect.h - 20.0).abs() < 1e-9);
    }

    // ── gutter_mark ──────────────────────────────────────────────────────────

    #[test]
    fn gutter_mark_paints_small_rect_at_expected_position() {
        let m = default_metrics(); // cell_w=10, cell_h=20
        let w = 400usize;
        let h = 200usize;
        let mut r = Raster::new(w, h);
        r.clear([0, 0, 0]);
        // Use pad_x=8, origin_x=10 so the mark falls at x = 10 - 8 + 2 = 4.
        r.pad_x = 8.0;
        r.origin_x = 10.0;
        r.origin_y = 0.0;

        // Draw a gutter mark on row 1.
        // Row 1 top = 20, cell center = 30; mark top = 30 - 3 = 27; [27, 33).
        // Mark x: 10 - 8 + 2 = 4; size 6 → [4, 10).
        let mark_rgb = [0xff, 0x00, 0x80];
        r.gutter_mark(m, 1, mark_rgb);

        let center_x = 7usize; // inside [4, 10)
        let center_y = 30usize; // inside [27, 33)
        assert_eq!(
            pixel_at(&r, center_x, center_y),
            mark_rgb,
            "center pixel must be mark color"
        );

        // Pixel just outside the mark to the left.
        assert_eq!(
            pixel_at(&r, 3, center_y),
            [0, 0, 0],
            "left of mark must be clear"
        );
        // Pixel just outside the mark above.
        assert_eq!(
            pixel_at(&r, center_x, 26),
            [0, 0, 0],
            "above mark must be clear"
        );
    }

    // ── fill_pixel_rect ───────────────────────────────────────────────────────

    #[test]
    fn fill_pixel_rect_paints_region() {
        let mut r = Raster::new(100, 100);
        r.clear([0, 0, 0]);
        r.fill_pixel_rect(10.0, 10.0, 20.0, 20.0, [0, 0, 255]);
        assert_eq!(pixel_at(&r, 15, 15), [0, 0, 255]);
        // Outside the rect should remain clear.
        assert_eq!(pixel_at(&r, 5, 5), [0, 0, 0]);
        assert_eq!(pixel_at(&r, 35, 35), [0, 0, 0]);
    }
}
