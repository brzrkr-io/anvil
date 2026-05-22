//! CoreText font layer for the terminal rasterizer.
//!
//! - Loads the embedded IBM Plex Mono Nerd Font TTF via in-memory
//!   `CTFontManager` registration.
//! - Provides `Font` with glyph metrics and per-codepoint glyph lookup.
//! - Implements `anvil_render::GlyphPainter` via `CoreTextPainter`, which
//!   creates a `CGBitmapContext` over the caller-supplied BGRA8 pixel buffer
//!   and calls `CTFontDrawGlyphs`.
//!
//! Port of `src/render/font.zig`.  Uses `objc2-core-text` and
//! `objc2-core-graphics` 0.3 typed bindings.
//!
//! # Coordinate notes
//!
//! `GlyphPainter::draw_glyph` receives `dest` in **top-down** bitmap space
//! (row 0 at the top), but `CTFontDrawGlyphs` draws into a CG context which
//! is **y-up** (origin at bottom-left).  The conversion is:
//!
//!   `cg_cell_bottom = bitmap_height - dest.y - dest.h`
//!
//! The baseline position within that CG cell is:
//!
//!   `baseline_y = cg_cell_bottom + metrics.descent`
//!
//! This matches the Zig raster's `cellRect` / `cellGlyph` geometry.

use std::ffi::c_void;
use std::ptr::NonNull;

use anvil_render::{FontMetrics, GlyphPainter, PixelRect};
use objc2_core_foundation::{CFError, CFRetained, CFString, CGPoint, CGSize};
use objc2_core_graphics::CGColorSpace;
#[allow(deprecated)]
use objc2_core_text::{CTFont, CTFontManagerRegisterGraphicsFont, CTFontOrientation};
use thiserror::Error;

// BGRA8, premultiplied alpha, little-endian 32-bit words — matches Metal
// MTLPixelFormatBGRA8Unorm (same constants as capi.zig).
const K_CG_IMAGE_ALPHA_PREMULTIPLIED_FIRST: u32 = 2;
const K_CG_BITMAP_BYTE_ORDER32_LITTLE: u32 = 2 << 12;
const BGRA8_BITMAP_INFO: u32 =
    K_CG_IMAGE_ALPHA_PREMULTIPLIED_FIRST | K_CG_BITMAP_BYTE_ORDER32_LITTLE;

/// The IBM Plex Mono build patched with developer icon glyphs (Nerd Font).
/// Bundled so the prompt's icons have glyphs regardless of system fonts.
static BUNDLED_FONT: &[u8] = include_bytes!("../../../src/assets/BlexMonoNerdFontMono-Regular.ttf");

/// Font loading errors.
#[derive(Debug, Error)]
pub enum FontError {
    #[error("CTFont creation failed")]
    FontCreateFailed,
    #[error("no font available from the provided list")]
    NoFontAvailable,
}

/// A CoreText font with pre-computed cell metrics.
pub struct Font {
    ct: CFRetained<CTFont>,
    pub metrics: FontMetrics,
}

// SAFETY: `CTFont` is an immutable CF object; its refcount is thread-safe
// (CF uses atomic retain/release internally).
unsafe impl Send for Font {}
unsafe impl Sync for Font {}

impl Font {
    /// Create a font from a Core Text family name (e.g. `"Menlo"`) at the
    /// given pixel size (point size × backing scale factor).
    pub fn init(name: &str, pixel_size: f64) -> Result<Font, FontError> {
        let cf_name = CFString::from_str(name);

        // SAFETY: `CTFont::with_name` is safe with a valid CFString; null matrix
        // means identity.  Returns a CFRetained that manages the CF retain count.
        let ct = unsafe { CTFont::with_name(&cf_name, pixel_size, std::ptr::null()) };

        let ascent = unsafe { ct.ascent() };
        let descent = unsafe { ct.descent() };
        let leading = unsafe { ct.leading() };

        // Cell width = advance of 'M' (monospace — any glyph works; 'M' is
        // a safe, always-present choice).
        let ch: u16 = b'M' as u16;
        let mut glyph: u16 = 0;
        // SAFETY: ch and glyph are valid u16 stack values; count = 1.
        unsafe {
            ct.glyphs_for_characters(
                NonNull::new(&ch as *const u16 as *mut u16).unwrap(),
                NonNull::new(&mut glyph as *mut u16).unwrap(),
                1,
            );
        }
        let mut adv = CGSize {
            width: 0.0,
            height: 0.0,
        };
        // SAFETY: glyph and adv are valid stack values; count = 1.
        unsafe {
            ct.advances_for_glyphs(
                CTFontOrientation::Default,
                NonNull::new(&glyph as *const u16 as *mut u16).unwrap(),
                &mut adv as *mut CGSize,
                1,
            );
        }

        let metrics = FontMetrics {
            cell_w: adv.width.ceil(),
            cell_h: (ascent + descent + leading).ceil(),
            descent,
        };

        Ok(Font { ct, metrics })
    }

    /// Try each name in order; return the first that loads with a non-zero
    /// cell width.
    pub fn init_first_available(names: &[&str], pixel_size: f64) -> Result<Font, FontError> {
        for &name in names {
            match Self::init(name, pixel_size) {
                Ok(f) if f.metrics.cell_w > 0.0 => return Ok(f),
                _ => continue,
            }
        }
        Err(FontError::NoFontAvailable)
    }

    /// The glyph index for a Unicode codepoint.  Returns 0 (missing glyph)
    /// when the font has no glyph for it.
    pub fn glyph(&self, cp: u32) -> u16 {
        let mut chars = [0u16; 2];
        let mut glyphs = [0u16; 2];
        let n: isize = if cp <= 0xFFFF {
            chars[0] = cp as u16;
            1
        } else {
            let v = cp - 0x10000;
            chars[0] = 0xD800 + (v >> 10) as u16;
            chars[1] = 0xDC00 + (v & 0x3FF) as u16;
            2
        };
        // SAFETY: chars and glyphs are valid stack buffers; n ≤ 2.
        unsafe {
            self.ct.glyphs_for_characters(
                NonNull::new(chars.as_mut_ptr()).unwrap(),
                NonNull::new(glyphs.as_mut_ptr()).unwrap(),
                n,
            );
        }
        glyphs[0]
    }
}

/// Register the bundled Nerd Font with CoreText so `CTFont::with_name` can
/// resolve it by family name.  Best-effort: on any failure the app falls back
/// to system fonts — never fatal.
pub fn register_bundled() {
    // SAFETY: CGDataProvider::with_data with a null release callback and a
    // 'static byte slice is safe; the data outlives the process.
    let provider = unsafe {
        objc2_core_graphics::CGDataProvider::with_data(
            std::ptr::null_mut(),
            BUNDLED_FONT.as_ptr() as *const c_void,
            BUNDLED_FONT.len(),
            None,
        )
    };
    let provider = match provider {
        Some(p) => p,
        None => {
            eprintln!("bundled font: CGDataProvider creation failed");
            return;
        }
    };

    let cg_font = objc2_core_graphics::CGFont::with_data_provider(&provider);
    let cg_font = match cg_font {
        Some(f) => f,
        None => {
            eprintln!("bundled font: CGFont creation failed");
            return;
        }
    };

    // SAFETY: cg_font is a valid +1 retained CGFont; error pointer is valid.
    let mut err_ptr: *mut CFError = std::ptr::null_mut();
    #[allow(deprecated)] // CTFontManagerRegisterGraphicsFont is deprecated but still the
    // correct in-memory registration path; the replacement APIs require a URL or data blob
    // workflow that isn't available in the process-scope registration we need.
    let ok = unsafe { CTFontManagerRegisterGraphicsFont(&cg_font, &mut err_ptr) };
    if !ok {
        if !err_ptr.is_null() {
            // SAFETY: err_ptr is a +1 retained CFError; drop via CFRetained.
            drop(unsafe { CFRetained::from_raw(NonNull::new(err_ptr).unwrap()) });
        }
        eprintln!("bundled font: CoreText registration failed");
    }
}

// ── GlyphPainter implementation ──────────────────────────────────────────────

/// Implements `GlyphPainter` via CoreText.  Stateless — each `draw_glyph`
/// call creates a temporary `CGBitmapContext` over the caller's pixel buffer
/// and draws through `CTFontDrawGlyphs`.
pub struct CoreTextPainter<'a> {
    font: &'a Font,
}

impl<'a> CoreTextPainter<'a> {
    pub fn new(font: &'a Font) -> Self {
        Self { font }
    }
}

impl GlyphPainter for CoreTextPainter<'_> {
    /// Draw the glyph for Unicode `codepoint` into the BGRA8 `pixels` buffer
    /// at the cell described by `dest` (top-down bitmap coordinates).
    ///
    /// The CoreText context is y-up, so we convert `dest.y` (top-down) to
    /// CG y-up before calling `CTFontDrawGlyphs`.
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
    ) {
        // `codepoint` is the cell's Unicode scalar. The GlyphPainter contract
        // passes codepoints, not pre-resolved glyph indices — resolve it
        // through the font cmap (this also handles astral-plane codepoints via
        // the surrogate path in `Font::glyph`).
        if codepoint == 0 {
            return;
        }
        let glyph: u16 = self.font.glyph(codepoint);
        if glyph == 0 {
            return; // the font has no glyph for this codepoint
        }

        let space = match CGColorSpace::new_device_rgb() {
            Some(s) => s,
            None => return,
        };

        // Create a CGBitmapContext backed by the caller's pixel buffer.
        // SAFETY:
        // - pixels.as_mut_ptr() is valid for bitmap_width * bitmap_height * 4 bytes.
        // - space is a valid CGColorSpace.
        // - BGRA8_BITMAP_INFO matches Metal's BGRA8Unorm layout.
        // - None release callback: CoreGraphics must NOT free the buffer.
        let ctx = unsafe {
            objc2_core_graphics::CGBitmapContextCreate(
                pixels.as_mut_ptr() as *mut c_void,
                bitmap_width,
                bitmap_height,
                8,
                bitmap_width * 4,
                Some(&space),
                BGRA8_BITMAP_INFO,
            )
        };
        let ctx = match ctx {
            Some(c) => c,
            None => return,
        };

        // Set identity text matrix (required by CoreText).
        // SAFETY: CGAffineTransformIdentity is a valid static extern value.
        objc2_core_graphics::CGContext::set_text_matrix(Some(&ctx), unsafe {
            objc2_core_graphics::CGAffineTransformIdentity
        });

        // Set the foreground fill colour.
        objc2_core_graphics::CGContext::set_rgb_fill_color(
            Some(&ctx),
            fg[0] as f64 / 255.0,
            fg[1] as f64 / 255.0,
            fg[2] as f64 / 255.0,
            1.0,
        );

        // Convert top-down dest.y to CG y-up:
        //   cg_cell_bottom = bitmap_height - dest.y - dest.h
        // Baseline within the CG cell:
        //   baseline_y = cg_cell_bottom + metrics.descent
        let cg_cell_bottom = bitmap_height as f64 - dest.y - dest.h;
        let baseline_y = cg_cell_bottom + metrics.descent;

        let pos = CGPoint {
            x: dest.x,
            y: baseline_y,
        };

        // SAFETY:
        // - glyph is a stack-allocated u16; NonNull is valid for one element.
        // - pos is a stack-allocated CGPoint; NonNull is valid for one element.
        // - ctx is a valid CGContext.
        unsafe {
            self.font.ct.draw_glyphs(
                NonNull::new(&glyph as *const u16 as *mut u16).unwrap(),
                NonNull::new(&pos as *const CGPoint as *mut CGPoint).unwrap(),
                1,
                &ctx,
            );
        }
        // `ctx` and `space` are dropped (CFRetained) here, releasing the
        // CoreFoundation refcount.  They do NOT free the pixel buffer because
        // we passed None as the release callback to CGBitmapContextCreate.
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Port of "brand mono font stack loads with sane metrics"
    #[test]
    fn brand_mono_font_stack_loads_with_sane_metrics() {
        let names = &["IBMPlexMono", "SFMono-Regular", "Menlo"];
        let f = Font::init_first_available(names, 26.0).unwrap();
        assert!(f.metrics.cell_w > 0.0);
        assert!(f.metrics.cell_h > 0.0);
        // Monospace at 26px: cell taller than wide, both within a sane range.
        assert!(f.metrics.cell_h > f.metrics.cell_w);
        assert!(f.metrics.cell_w < 64.0 && f.metrics.cell_h < 64.0);
    }

    /// Port of "glyph lookup resolves common characters"
    #[test]
    fn glyph_lookup_resolves_common_characters() {
        let names = &["IBMPlexMono", "SFMono-Regular", "Menlo"];
        let f = Font::init_first_available(names, 26.0).unwrap();
        assert!(f.glyph('A' as u32) != 0);
        assert!(f.glyph('z' as u32) != 0);
        assert!(f.glyph('0' as u32) != 0);
    }

    /// Port of "glyph handles an astral-plane codepoint via the surrogate-pair path"
    #[test]
    fn glyph_handles_astral_plane_codepoint() {
        let names = &["IBMPlexMono", "SFMono-Regular", "Menlo"];
        let f = Font::init_first_available(names, 26.0).unwrap();
        // U+1F600 is above the BMP; the lookup must not crash.
        let _ = f.glyph(0x1F600);
    }

    /// Port of "initFirstAvailable with no names returns NoFontAvailable"
    #[test]
    fn init_first_available_empty_returns_error() {
        let result = Font::init_first_available(&[], 26.0);
        assert!(matches!(result, Err(FontError::NoFontAvailable)));
    }

    /// `draw_glyph` receives a Unicode codepoint and must resolve it through
    /// the font cmap. Regression: it once used the codepoint directly as a
    /// glyph index, so every character drew the wrong glyph.
    #[test]
    fn draw_glyph_resolves_codepoint_through_the_cmap() {
        let names = &["IBMPlexMono", "SFMono-Regular", "Menlo"];
        let font = Font::init_first_available(names, 26.0).unwrap();
        let mut painter = CoreTextPainter::new(&font);
        let w = font.metrics.cell_w.ceil() as usize;
        let h = font.metrics.cell_h.ceil() as usize;
        let metrics = font.metrics;
        let dest = PixelRect {
            x: 0.0,
            y: 0.0,
            w: w as f64,
            h: h as f64,
        };

        let mut ink = |cp: u32| -> usize {
            let mut buf = vec![0u8; w * h * 4];
            painter.draw_glyph(cp, dest, [255, 255, 255], metrics, &mut buf, w, h);
            buf.iter().filter(|&&b| b != 0).count()
        };

        // 'M' is a dense glyph — it must ink a substantial number of pixels.
        assert!(ink('M' as u32) > 0, "drawing 'M' inked no pixels");
        // The space glyph is blank — it must ink nothing. With the
        // codepoint-as-glyph-index bug, U+0020 drew glyph #32 (a letter).
        let space_ink = ink(' ' as u32);
        assert_eq!(
            space_ink, 0,
            "drawing space inked {space_ink} pixels — codepoint was not resolved through the cmap"
        );
    }
}
