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

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;

use anvil_render::{FontMetrics, GlyphPainter, PixelRect};
use objc2_core_foundation::{CFError, CFRetained, CFString, CGPoint, CGSize};
#[allow(deprecated)]
use objc2_core_text::{CTFont, CTFontManagerRegisterGraphicsFont, CTFontOrientation};
use thiserror::Error;

/// The IBM Plex Mono build patched with developer icon glyphs (Nerd Font).
/// Bundled so the prompt's icons have glyphs regardless of system fonts.
static BUNDLED_FONT: &[u8] = include_bytes!("../../../assets/BlexMonoNerdFontMono-Regular.ttf");

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

/// Cell-sized grayscale rasterization context used to bake glyphs into alpha
/// masks once. Owned by `CoreTextPainter`; rebuilt when cell dimensions change.
struct Rasterizer {
    ctx: CFRetained<objc2_core_graphics::CGContext>,
    buf: Vec<u8>,
    cell_w: usize,
    cell_h: usize,
    descent: f64,
}

/// A pre-rasterized alpha mask for a single glyph, sized to the cell.
struct GlyphMask {
    pixels: Vec<u8>,
    width: usize,
    height: usize,
}

/// Implements `GlyphPainter` via CoreText with a **glyph mask cache**.
///
/// `CTFontDrawGlyphs` is the dominant cost in the render loop (~50–200µs per
/// call); a steady-state frame redraws ~1,900 cells and calls it for each.
/// We rasterize each glyph once into a small alpha mask, then composite the
/// mask tinted with the foreground color on subsequent draws — the inner
/// loop is a tight CPU alpha-blend that skips zero-alpha pixels (the common
/// case in printable glyphs). Steady-state typing is then bounded by memory
/// bandwidth, not by CoreText.
pub struct CoreTextPainter<'a> {
    font: &'a Font,
    rasterizer: Option<Rasterizer>,
    cache: HashMap<u16, GlyphMask>,
}

impl<'a> CoreTextPainter<'a> {
    pub fn new(font: &'a Font) -> Self {
        Self {
            font,
            rasterizer: None,
            cache: HashMap::new(),
        }
    }
}

impl GlyphPainter for CoreTextPainter<'_> {
    /// Draw the glyph for Unicode `codepoint` into the BGRA8 `pixels` buffer
    /// at the cell described by `dest` (top-down bitmap coordinates).
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
        if codepoint == 0 {
            return;
        }
        let glyph: u16 = self.font.glyph(codepoint);
        if glyph == 0 {
            return;
        }

        // Masks are sized to the cell. (Re)build the rasterizer if cell
        // dimensions change; invalidate the cache when it does.
        let cell_w = dest.w.round() as usize;
        let cell_h = dest.h.round() as usize;
        if cell_w == 0 || cell_h == 0 {
            return;
        }
        let need_new = self
            .rasterizer
            .as_ref()
            .is_none_or(|r| r.cell_w != cell_w || r.cell_h != cell_h);
        if need_new {
            self.cache.clear();
            self.rasterizer = Rasterizer::new(cell_w, cell_h, metrics.descent);
            if self.rasterizer.is_none() {
                return;
            }
        }

        // Rasterize the glyph once; subsequent calls hit the cache.
        if !self.cache.contains_key(&glyph) {
            let mask = self
                .rasterizer
                .as_mut()
                .unwrap()
                .rasterize(&self.font.ct, glyph);
            self.cache.insert(glyph, mask);
        }
        let mask = self.cache.get(&glyph).unwrap();

        // Composite the mask into the BGRA8 destination, tinted with fg.
        composite_mask(
            mask,
            dest.x,
            dest.y,
            fg,
            pixels,
            bitmap_width,
            bitmap_height,
        );
    }
}

impl Rasterizer {
    fn new(cell_w: usize, cell_h: usize, descent: f64) -> Option<Self> {
        let mut buf = vec![0u8; cell_w * cell_h];
        let gray = objc2_core_graphics::CGColorSpace::new_device_gray()?;
        // SAFETY:
        // - `buf` is valid for cell_w * cell_h bytes and lives in this
        //   struct alongside the context (drop order: ctx before buf).
        // - gray is a valid CGColorSpace.
        // - bitmapInfo = 0 = kCGImageAlphaNone, an 8-bit single-channel
        //   gray context: the rendered glyph's luminance == its coverage.
        // - None release callback: CG never frees `buf`.
        let buf_ptr = buf.as_mut_ptr() as *mut c_void;
        let ctx = unsafe {
            objc2_core_graphics::CGBitmapContextCreate(
                buf_ptr,
                cell_w,
                cell_h,
                8,
                cell_w,
                Some(&gray),
                0, // kCGImageAlphaNone
            )
        }?;
        // SAFETY: CGAffineTransformIdentity is a valid static extern.
        objc2_core_graphics::CGContext::set_text_matrix(Some(&ctx), unsafe {
            objc2_core_graphics::CGAffineTransformIdentity
        });
        // White fill: the glyph rasterizes at full luminance = full coverage.
        objc2_core_graphics::CGContext::set_rgb_fill_color(Some(&ctx), 1.0, 1.0, 1.0, 1.0);
        drop(gray);
        Some(Self {
            ctx,
            buf,
            cell_w,
            cell_h,
            descent,
        })
    }

    fn rasterize(&mut self, ct: &CTFont, glyph: u16) -> GlyphMask {
        // Clear the cell-sized buffer to fully transparent.
        self.buf.fill(0);
        // Baseline in CG y-up = `descent` pixels above the bottom of the cell.
        let pos = CGPoint {
            x: 0.0,
            y: self.descent,
        };
        // SAFETY: glyph and pos are stack values valid for one element each;
        // `self.ctx` is a valid CGContext over `self.buf`.
        unsafe {
            ct.draw_glyphs(
                NonNull::new(&glyph as *const u16 as *mut u16).unwrap(),
                NonNull::new(&pos as *const CGPoint as *mut CGPoint).unwrap(),
                1,
                &self.ctx,
            );
        }
        GlyphMask {
            pixels: self.buf.clone(),
            width: self.cell_w,
            height: self.cell_h,
        }
    }
}

/// Alpha-blend `mask` into the BGRA8 destination at top-down (`dest_x`,
/// `dest_y`), tinted with `fg`. Zero-alpha pixels short-circuit — most of a
/// printable glyph's cell is empty space, so the inner loop is cheap.
fn composite_mask(
    mask: &GlyphMask,
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
    for y in 0..mask.height {
        let py = dy + y as isize;
        if py < 0 || (py as usize) >= dst_h {
            continue;
        }
        let row_start = y * mask.width;
        let mask_row = &mask.pixels[row_start..row_start + mask.width];
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
            // BGRA layout, destination treated as opaque.
            dst[off] = ((dst[off] as u32 * inv + fg_b * aa + 127) / 255) as u8;
            dst[off + 1] = ((dst[off + 1] as u32 * inv + fg_g * aa + 127) / 255) as u8;
            dst[off + 2] = ((dst[off + 2] as u32 * inv + fg_r * aa + 127) / 255) as u8;
        }
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
