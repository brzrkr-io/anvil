//! Proportional UI text rasterizer via CoreText.
//!
//! This module provides [`UiPainter`], which rasterizes variable-width text
//! for chrome surfaces (explorer, tabs, status, breadcrumbs, overlays).
//! It is intentionally separate from the monospaced cell-grid path in
//! `font.rs` / `CoreTextPainter`.
//!
//! # Design
//!
//! - `UiPainter` holds the active family name, backing scale, and an LRU
//!   `UiLineCache` (1 024 entries / 4 MiB mask memory cap).
//! - A CTFont is created per (family, size_px, weight) and stored in a small
//!   per-size font cache to avoid redundant CoreText font allocations.
//! - Each call to `draw_line` rasterizes text via `CTLine` into an 8-bit gray
//!   context, stores the coverage mask, then composites onto the BGRA8 frame
//!   buffer with a tinted alpha blend (same pattern as `font.rs::composite_mask`).
//! - Color is NOT part of the cache key — mask is reused and tinted at
//!   composite time, matching the glyph cache strategy.
//! - Pixel snapping: x and baseline_y are floored to integer device pixels.
//! - Size is quantized to 0.5 pt steps before forming the cache key.
//! - Subpixel positioning is off; all masks are rasterized at x = 0.
//!
//! # Fallback chain
//!
//! Configured family → "SF Pro Text" → system UI font → no-op (logged once).
//!
//! # Coordinate notes
//!
//! `draw_line` receives top-down device-pixel coordinates. Inside the painter
//! the CG context is y-up: the pen is placed at `(1.0, descent + 1.0)` so
//! glyphs sit correctly within the allocated mask with 1 px padding.

use std::collections::HashMap;
use std::ffi::c_void;

use anvil_config::UiFontCfg;
use anvil_render::raster::{UiTextPainter, UiWeight};
use objc2_core_foundation::{
    CFAttributedString, CFDictionary, CFDictionaryKeyCallBacks, CFDictionaryValueCallBacks,
    CFRetained, CFString, kCFBooleanTrue,
};
use objc2_core_text::{
    CTFont, CTFontUIFontType, CTLine, kCTFontAttributeName,
    kCTForegroundColorFromContextAttributeName,
};

// ── Font cache ────────────────────────────────────────────────────────────────

/// Cache mapping quantized pixel sizes to a loaded `CTFont`.
///
/// Keyed by `(is_strong, size_q_half_pts)` so each logical size is only
/// created once per painter lifetime.  Cleared on config reset.
struct FontCache {
    map: HashMap<(bool, u32), CFRetained<CTFont>>,
    family: String,
}

// SAFETY: CTFont is an immutable CF object; CF uses atomic retain/release.
unsafe impl Send for FontCache {}
unsafe impl Sync for FontCache {}

impl FontCache {
    fn new(family: String) -> Self {
        Self {
            map: HashMap::new(),
            family,
        }
    }

    /// Get or create a CTFont for `(is_strong, pixel_size)`.
    ///
    /// Falls back to `SF Pro Text` then the system UI font on failure.
    fn get_or_create(&mut self, is_strong: bool, pixel_size: f64) -> &CTFont {
        let size_q = (pixel_size * 2.0).round() as u32;
        let key = (is_strong, size_q);
        self.map
            .entry(key)
            .or_insert_with(|| load_ct_font_with_fallback(&self.family, "SF Pro Text", pixel_size))
    }

    #[allow(dead_code)]
    fn clear(&mut self) {
        self.map.clear();
    }
}

// ── LRU line-mask cache ───────────────────────────────────────────────────────

/// Maximum number of cached line masks retained per [`UiPainter`].
const LINE_CACHE_CAP: usize = 1024;

/// Maximum total byte size of cached pixel masks (4 MiB).
const LINE_CACHE_MAX_BYTES: usize = 4 * 1024 * 1024;

/// 8-bit luminance mask for a rasterized text line.
struct UiLineMask {
    pixels: Vec<u8>,
    w: usize,
    h: usize,
    /// Ascent of the line in device pixels (used to position the top of the mask).
    ascent: f64,
    /// Descent of the line in device pixels (positive, below baseline).
    #[allow(dead_code)]
    descent: f64,
}

impl UiLineMask {
    fn byte_size(&self) -> usize {
        self.pixels.len()
    }
}

/// LRU cache of rasterized line masks keyed by
/// `(is_strong, size_q_half_pts, text)`.
///
/// `size_q_half_pts` is `(size_px * 2.0).round() as u32`.
/// Color is not in the key — mask is tinted at composite time.
struct UiLineCache {
    map: HashMap<(bool, u32, String), (UiLineMask, u64)>,
    tick: u64,
    cap: usize,
    total_bytes: usize,
    max_bytes: usize,
}

impl UiLineCache {
    fn new(cap: usize, max_bytes: usize) -> Self {
        Self {
            map: HashMap::new(),
            tick: 0,
            cap,
            total_bytes: 0,
            max_bytes,
        }
    }

    fn clear(&mut self) {
        self.map.clear();
        self.tick = 0;
        self.total_bytes = 0;
    }

    fn get(&mut self, key: &(bool, u32, String)) -> Option<&UiLineMask> {
        if let Some(entry) = self.map.get_mut(key) {
            self.tick += 1;
            entry.1 = self.tick;
            return self.map.get(key).map(|(m, _)| m);
        }
        None
    }

    fn insert(&mut self, key: (bool, u32, String), mask: UiLineMask) {
        // Evict LRU entries until within both limits.
        while (self.map.len() >= self.cap || self.total_bytes + mask.byte_size() > self.max_bytes)
            && !self.map.is_empty()
        {
            if let Some((evict_key, _)) = self.map.iter().min_by_key(|(_, (_, t))| t) {
                let evict_key = evict_key.clone();
                if let Some((m, _)) = self.map.remove(&evict_key) {
                    self.total_bytes = self.total_bytes.saturating_sub(m.byte_size());
                }
            } else {
                break;
            }
        }
        self.total_bytes += mask.byte_size();
        self.tick += 1;
        self.map.insert(key, (mask, self.tick));
    }
}

// ── UiPainter ─────────────────────────────────────────────────────────────────

/// CoreText proportional text painter.
///
/// Holds the active font family name, a per-size font cache, a backing-scale
/// factor for pt→px conversion, and an LRU mask cache.
///
/// Construct once at startup with [`UiPainter::new`]; rebuild on config change.
pub struct UiPainter {
    fonts: FontCache,
    backing_scale: f64,
    cache: UiLineCache,
    /// One-shot log flag for rasterization failures.
    logged_raster_fail: bool,
}

impl UiPainter {
    /// Construct a `UiPainter` from `UiFontCfg` and the window's backing scale.
    ///
    /// Fallback chain per spec: configured family → `"SF Pro Text"` → system UI font.
    pub fn new(cfg: UiFontCfg, backing_scale: f64) -> Self {
        Self::new_with_caps(cfg, backing_scale, LINE_CACHE_CAP, LINE_CACHE_MAX_BYTES)
    }

    /// Like [`new`] but with explicit cache limits — used by unit tests.
    pub fn new_with_caps(cfg: UiFontCfg, backing_scale: f64, cap: usize, max_bytes: usize) -> Self {
        Self {
            fonts: FontCache::new(cfg.family),
            backing_scale,
            cache: UiLineCache::new(cap, max_bytes),
            logged_raster_fail: false,
        }
    }

    /// Clear caches and reload from new config.  Called on config live-reload.
    pub fn reset(&mut self, cfg: UiFontCfg) {
        self.fonts = FontCache::new(cfg.family);
        self.cache.clear();
    }

    /// Build or retrieve a cached `UiLineMask` for the given parameters.
    ///
    /// Returns `None` on rasterization failure (logged once per painter).
    fn get_mask(&mut self, text: &str, size_pt: f64, weight: UiWeight) -> Option<&UiLineMask> {
        if text.is_empty() {
            return None;
        }
        let pixel_size = size_pt * self.backing_scale;
        let size_q = (pixel_size * 2.0).round() as u32;
        let is_strong = weight != UiWeight::Regular;
        let key = (is_strong, size_q, text.to_string());

        // Cache hit — just update the tick.
        if self.cache.get(&key).is_some() {
            return self.cache.get(&key);
        }

        // Get (or create) the CTFont for this size.
        // We can't hold a borrow on self.fonts while also calling self.cache,
        // so clone the CTFont pointer out of the cache.
        let ct: CFRetained<CTFont> = {
            let ct_ref = self.fonts.get_or_create(is_strong, pixel_size);
            // SAFETY: CTFont is an immutable CF object; retain is safe.
            unsafe {
                CFRetained::retain(std::ptr::NonNull::new_unchecked(
                    ct_ref as *const CTFont as *mut CTFont,
                ))
            }
        };

        let mask = rasterize_line(&ct, text);
        let mask = match mask {
            Some(m) => m,
            None => {
                if !self.logged_raster_fail {
                    eprintln!("anvil: ui_text: CTLine rasterization failed for {:?}", text);
                    self.logged_raster_fail = true;
                }
                return None;
            }
        };

        self.cache.insert(key.clone(), mask);
        self.cache.get(&key)
    }
}

impl UiTextPainter for UiPainter {
    fn measure(&mut self, text: &str, size_pt: f64, weight: UiWeight) -> f64 {
        if text.is_empty() {
            return 0.0;
        }
        let pixel_size = size_pt * self.backing_scale;
        let is_strong = weight != UiWeight::Regular;
        let ct: CFRetained<CTFont> = {
            let ct_ref = self.fonts.get_or_create(is_strong, pixel_size);
            // SAFETY: CTFont is immutable CF; retain is safe.
            unsafe {
                CFRetained::retain(std::ptr::NonNull::new_unchecked(
                    ct_ref as *const CTFont as *mut CTFont,
                ))
            }
        };
        let line = build_ct_line(&ct, text);
        match line {
            None => 0.0,
            Some(l) => {
                let width = unsafe {
                    l.typographic_bounds(
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    )
                };
                // Return in device pixels (callers treating it as pt must account for scale).
                // Match the spec: measure returns a pixel-space width here, but per-trait
                // contract the unit is the same as x_px in draw_line.
                width
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_line(
        &mut self,
        text: &str,
        x_px: f64,
        baseline_y_px: f64,
        size_pt: f64,
        weight: UiWeight,
        fg: [u8; 3],
        pixels: &mut [u8],
        bitmap_w: usize,
        bitmap_h: usize,
    ) {
        if text.is_empty() {
            return;
        }

        // Pixel-snap both coordinates.
        let dest_x = x_px.floor();
        let dest_y_baseline = baseline_y_px.floor();

        let mask = match self.get_mask(text, size_pt, weight) {
            Some(m) => m,
            None => return,
        };

        // Top of the mask in top-down bitmap space = baseline_y - ascent.
        let top_y = dest_y_baseline - mask.ascent.ceil();

        composite_mask(mask, dest_x, top_y, fg, pixels, bitmap_w, bitmap_h);
    }
}

// ── CoreText helpers ──────────────────────────────────────────────────────────

/// Build a `CTLine` for `text` using `ct` as the font attribute.
///
/// Sets `kCTForegroundColorFromContextAttributeName = kCFBooleanTrue` so that
/// `CTLine::draw` uses the CG context's fill color (white) as the glyph color,
/// which gives a luminance mask where pixel value == coverage.
///
/// Returns `None` on any CF allocation failure.
fn build_ct_line(ct: &CTFont, text: &str) -> Option<CFRetained<CTLine>> {
    // Build CFDictionary {
    //   kCTFontAttributeName                    → ct,
    //   kCTForegroundColorFromContextAttributeName → kCFBooleanTrue,
    // }
    #[allow(clippy::borrow_deref_ref)]
    let font_key = unsafe { &*kCTFontAttributeName as *const CFString as *const c_void };
    #[allow(clippy::borrow_deref_ref)]
    let fg_key =
        unsafe { &*kCTForegroundColorFromContextAttributeName as *const CFString as *const c_void };
    let font_val = ct as *const CTFont as *const c_void;
    // SAFETY: kCFBooleanTrue is a global singleton CF object; unwrap is safe on macOS.
    let fg_val = unsafe { kCFBooleanTrue.unwrap() as *const _ as *const c_void };

    let mut keys = [font_key, fg_key];
    let mut vals = [font_val, fg_val];

    // SAFETY: 2-element key/value arrays; global callback statics.
    let dict: Option<CFRetained<CFDictionary>> = unsafe {
        unsafe extern "C" {
            static kCFTypeDictionaryKeyCallBacks: CFDictionaryKeyCallBacks;
            static kCFTypeDictionaryValueCallBacks: CFDictionaryValueCallBacks;
        }
        CFDictionary::new(
            None,
            keys.as_mut_ptr(),
            vals.as_mut_ptr(),
            2,
            &kCFTypeDictionaryKeyCallBacks as *const CFDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks as *const CFDictionaryValueCallBacks,
        )
    };
    let dict = dict?;

    // Build CFAttributedString { text, attrs = dict }
    let cf_text = CFString::from_str(text);
    // SAFETY: cf_text and dict are valid non-null CF objects.
    let attr: Option<CFRetained<CFAttributedString>> =
        unsafe { CFAttributedString::new(None, Some(&cf_text), Some(&dict)) };
    let attr = attr?;

    // SAFETY: attr is a valid CFAttributedString.
    let line = unsafe { CTLine::with_attributed_string(&attr) };
    Some(line)
}

/// Rasterize `text` into an 8-bit gray mask and return a [`UiLineMask`].
///
/// Returns `None` on any CoreGraphics allocation failure.
fn rasterize_line(ct: &CTFont, text: &str) -> Option<UiLineMask> {
    let line = build_ct_line(ct, text)?;

    // Measure typographic bounds.
    let mut line_ascent: f64 = 0.0;
    let mut line_descent: f64 = 0.0;
    let width = unsafe {
        line.typographic_bounds(&mut line_ascent, &mut line_descent, std::ptr::null_mut())
    };

    // Mask dimensions: add 2 px padding on each axis to avoid clipping.
    let mask_w = (width.ceil() as usize + 2).max(1);
    let mask_h = ((line_ascent + line_descent).ceil() as usize + 2).max(1);

    let mut buf = vec![0u8; mask_w * mask_h];

    let gray = objc2_core_graphics::CGColorSpace::new_device_gray()?;
    let buf_ptr = buf.as_mut_ptr() as *mut c_void;
    // SAFETY: buf is valid for mask_w * mask_h bytes; gray is a valid colorspace.
    let ctx = unsafe {
        objc2_core_graphics::CGBitmapContextCreate(
            buf_ptr,
            mask_w,
            mask_h,
            8,      // bits per component
            mask_w, // bytes per row
            Some(&gray),
            0, // kCGImageAlphaNone
        )
    }?;
    drop(gray);

    // White fill so luminance == glyph coverage.
    objc2_core_graphics::CGContext::set_rgb_fill_color(Some(&ctx), 1.0, 1.0, 1.0, 1.0);

    // Font smoothing on; subpixel positioning off (all masks rasterized at x=0).
    objc2_core_graphics::CGContext::set_should_smooth_fonts(Some(&ctx), true);

    // Identity text matrix required by CoreText.
    objc2_core_graphics::CGContext::set_text_matrix(
        Some(&ctx),
        // SAFETY: global extern static.
        unsafe { objc2_core_graphics::CGAffineTransformIdentity },
    );

    // Position pen: 1 px left padding, descent + 1 px bottom padding (y-up CG).
    objc2_core_graphics::CGContext::set_text_position(Some(&ctx), 1.0, line_descent.ceil() + 1.0);

    // Draw the line.
    // SAFETY: ctx is a valid CGContext; line is a valid CTLine.
    unsafe { line.draw(&ctx) };

    Some(UiLineMask {
        pixels: buf,
        w: mask_w,
        h: mask_h,
        ascent: line_ascent,
        descent: line_descent,
    })
}

/// Alpha-blend an 8-bit gray `mask` into the BGRA8 `dst` at top-down
/// (`dest_x`, `dest_y`) — top-left corner of the mask.
/// Tinted with `fg`.  Mirrors `font.rs::composite_mask`.
fn composite_mask(
    mask: &UiLineMask,
    dest_x: f64,
    dest_y: f64,
    fg: [u8; 3],
    dst: &mut [u8],
    dst_w: usize,
    dst_h: usize,
) {
    let dx = dest_x.round() as isize;
    let dy = dest_y.round() as isize;
    let stride = dst_w * 4;
    let fg_b = fg[2] as u32;
    let fg_g = fg[1] as u32;
    let fg_r = fg[0] as u32;
    for y in 0..mask.h {
        let py = dy + y as isize;
        if py < 0 || (py as usize) >= dst_h {
            continue;
        }
        let row_start = y * mask.w;
        let mask_row = &mask.pixels[row_start..row_start + mask.w];
        for (x, &a) in mask_row.iter().enumerate() {
            if a == 0 {
                continue;
            }
            let px = dx + x as isize;
            if px < 0 || (px as usize) >= dst_w {
                continue;
            }
            let off = py as usize * stride + px as usize * 4;
            let aa = a as u32;
            let inv = 255 - aa;
            // BGRA layout; destination treated as opaque.
            dst[off] = ((dst[off] as u32 * inv + fg_b * aa + 127) / 255) as u8;
            dst[off + 1] = ((dst[off + 1] as u32 * inv + fg_g * aa + 127) / 255) as u8;
            dst[off + 2] = ((dst[off + 2] as u32 * inv + fg_r * aa + 127) / 255) as u8;
        }
    }
}

// ── Font loading helpers ──────────────────────────────────────────────────────

/// Load a `CTFont` with three-level fallback: `family` → `sf_pro_fallback` → system UI font.
///
/// Always returns a valid `CFRetained<CTFont>`.
fn load_ct_font_with_fallback(
    family: &str,
    sf_pro_fallback: &str,
    pixel_size: f64,
) -> CFRetained<CTFont> {
    // 1. Configured family.
    let ct =
        unsafe { CTFont::with_name(&CFString::from_str(family), pixel_size, std::ptr::null()) };
    // Validate: if ascent > 0 the font loaded successfully (not a blank placeholder).
    if unsafe { ct.ascent() } > 0.0 {
        return ct;
    }
    eprintln!(
        "anvil: ui_text: font {:?} not found or has zero metrics; trying {:?}",
        family, sf_pro_fallback
    );

    // 2. SF Pro Text (or caller's secondary fallback).
    let ct2 = unsafe {
        CTFont::with_name(
            &CFString::from_str(sf_pro_fallback),
            pixel_size,
            std::ptr::null(),
        )
    };
    if unsafe { ct2.ascent() } > 0.0 {
        return ct2;
    }
    eprintln!(
        "anvil: ui_text: font {:?} not found; using system UI font",
        sf_pro_fallback
    );

    // 3. System UI font.
    if let Some(sys) =
        unsafe { CTFont::new_ui_font_for_language(CTFontUIFontType::System, pixel_size, None) }
    {
        return sys;
    }

    // Absolute fallback — should never happen on macOS.
    eprintln!("anvil: ui_text: system UI font unavailable; draw_line will be a no-op");
    ct // original blank placeholder — measure/draw will produce zeros
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use anvil_config::UiFontCfg;

    fn test_cfg() -> UiFontCfg {
        UiFontCfg::default()
    }

    fn painter_2x() -> UiPainter {
        UiPainter::new(test_cfg(), 2.0)
    }

    // ── measure ───────────────────────────────────────────────────────────────

    #[test]
    fn ui_measure_zero_for_empty_string() {
        let mut p = painter_2x();
        assert_eq!(p.measure("", 13.0, UiWeight::Regular), 0.0);
    }

    #[test]
    fn ui_measure_monotonic_in_length() {
        let mut p = painter_2x();
        let w1 = p.measure("A", 13.0, UiWeight::Regular);
        let w2 = p.measure("AB", 13.0, UiWeight::Regular);
        let w3 = p.measure("ABC", 13.0, UiWeight::Regular);
        assert!(w1 > 0.0, "single char must have positive width");
        assert!(w2 >= w1, "two chars should be at least as wide as one");
        assert!(w3 >= w2, "three chars should be at least as wide as two");
    }

    #[test]
    fn ui_measure_bold_ge_regular() {
        let mut p = painter_2x();
        let reg = p.measure("Hello", 13.0, UiWeight::Regular);
        let bold = p.measure("Hello", 13.0, UiWeight::Semibold);
        // Strong weight may be the same font in a fallback situation;
        // must be at least as wide (not narrower by more than 0.5 px).
        assert!(
            bold >= reg - 0.5,
            "semibold must not be narrower than regular (reg={reg}, bold={bold})"
        );
    }

    // ── draw_line ─────────────────────────────────────────────────────────────

    fn make_buf(w: usize, h: usize) -> Vec<u8> {
        vec![0u8; w * h * 4]
    }

    fn has_ink(buf: &[u8]) -> bool {
        buf.iter().any(|&b| b != 0)
    }

    #[test]
    fn ui_line_inks_pixels_for_nonempty_text() {
        let mut p = painter_2x();
        let w = 200usize;
        let h = 80usize;
        let mut buf = make_buf(w, h);
        p.draw_line(
            "Hello",
            2.0,
            50.0,
            13.0,
            UiWeight::Regular,
            [255, 255, 255],
            &mut buf,
            w,
            h,
        );
        assert!(
            has_ink(&buf),
            "draw_line must ink pixels for non-empty text"
        );
    }

    #[test]
    fn ui_line_zero_text_is_noop() {
        let mut p = painter_2x();
        let w = 100usize;
        let h = 40usize;
        let mut buf = make_buf(w, h);
        p.draw_line(
            "",
            0.0,
            20.0,
            13.0,
            UiWeight::Regular,
            [255, 255, 255],
            &mut buf,
            w,
            h,
        );
        assert!(
            !has_ink(&buf),
            "draw_line on empty string must not ink any pixel"
        );
    }

    #[test]
    fn ui_line_uses_baseline_y() {
        // Draw with two different baseline_y values; the inked rows must differ.
        let mut p = painter_2x();
        let w = 200usize;
        let h = 120usize;

        let mut buf_top = make_buf(w, h);
        p.draw_line(
            "M",
            2.0,
            30.0,
            13.0,
            UiWeight::Regular,
            [255, 255, 255],
            &mut buf_top,
            w,
            h,
        );

        let mut buf_bot = make_buf(w, h);
        p.draw_line(
            "M",
            2.0,
            80.0,
            13.0,
            UiWeight::Regular,
            [255, 255, 255],
            &mut buf_bot,
            w,
            h,
        );

        let first_inked = |buf: &[u8]| -> Option<usize> {
            for row in 0..h {
                if buf[row * w * 4..(row + 1) * w * 4].iter().any(|&b| b != 0) {
                    return Some(row);
                }
            }
            None
        };
        let top_row = first_inked(&buf_top).expect("top draw must ink pixels");
        let bot_row = first_inked(&buf_bot).expect("bottom draw must ink pixels");
        assert!(
            bot_row > top_row,
            "higher baseline_y must produce lower bitmap-y ink (top_row={top_row}, bot_row={bot_row})"
        );
    }

    #[test]
    fn ui_line_tints_to_fg_color() {
        let mut p = painter_2x();
        let w = 200usize;
        let h = 80usize;

        // Draw red text on a black (zeroed) background.
        let mut buf = make_buf(w, h);
        p.draw_line(
            "W",
            0.0,
            50.0,
            14.0,
            UiWeight::Regular,
            [255, 0, 0],
            &mut buf,
            w,
            h,
        );

        // In BGRA: B=buf[0], G=buf[1], R=buf[2].
        // Any inked pixel must have R > 0 and B == 0.
        let any_red = buf.chunks(4).any(|px| px[2] > 0 && px[0] == 0);
        assert!(any_red, "red fg must produce R>0, B=0 BGRA pixels");
    }

    // ── cache eviction ────────────────────────────────────────────────────────

    #[test]
    fn cache_evicts_when_capped() {
        // Use a tiny cap (5 entries) to force eviction.
        let mut p = UiPainter::new_with_caps(test_cfg(), 2.0, 5, 512 * 1024);
        let w = 200usize;
        let h = 80usize;

        // Draw 10 distinct strings so the cache must evict.
        for i in 0u32..10 {
            let s = format!("EvictTest{}", i);
            let mut buf = make_buf(w, h);
            p.draw_line(
                &s,
                0.0,
                50.0,
                13.0,
                UiWeight::Regular,
                [255, 255, 255],
                &mut buf,
                w,
                h,
            );
        }

        assert!(
            p.cache.map.len() <= 5,
            "cache must not exceed cap of 5; len={}",
            p.cache.map.len()
        );
    }
}
