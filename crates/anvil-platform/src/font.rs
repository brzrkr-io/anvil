//! CoreText font layer for the terminal rasterizer.
//!
//! - Loads the embedded IBM Plex Mono Nerd Font TTF via in-memory
//!   `CTFontManager` registration.
//! - Provides `Font` with glyph metrics and per-codepoint glyph lookup.
//! - Implements `anvil_render::GlyphPainter` via `CoreTextPainter`, which
//!   creates a `CGBitmapContext` over the caller-supplied BGRA8 pixel buffer
//!   and calls `CTFontDrawGlyphs`.
//!
//! Uses `objc2-core-text` and `objc2-core-graphics` 0.3 typed bindings.
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

// ── Inline LRU glyph cache ────────────────────────────────────────────────────

/// Maximum number of rasterized glyph masks retained per `CoreTextPainter`.
/// At a typical 8×16 cell the cap costs ≤ 256 KB; an uncapped u16 key space
/// could reach ~8 MB for an emoji-heavy session.
const GLYPH_CACHE_CAP: usize = 2048;

/// A tiny LRU cache mapping glyph index → `GlyphMask`.
///
/// Each entry carries a monotonic `tick` that is bumped on every hit or insert.
/// When the map is full, one linear scan evicts the entry with the smallest tick.
struct GlyphCache {
    map: HashMap<u16, (GlyphMask, u64)>,
    tick: u64,
    cap: usize,
}

impl GlyphCache {
    fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            tick: 0,
            cap,
        }
    }

    fn clear(&mut self) {
        self.map.clear();
        self.tick = 0;
    }

    /// Return a reference to the cached mask, bumping its recency tick.
    fn get(&mut self, key: u16) -> Option<&GlyphMask> {
        if let Some(entry) = self.map.get_mut(&key) {
            self.tick += 1;
            entry.1 = self.tick;
            // Re-borrow immutably for the return value.
            return self.map.get(&key).map(|(m, _)| m);
        }
        None
    }

    /// Insert a mask. Evicts the LRU entry when the cap is reached.
    fn insert(&mut self, key: u16, mask: GlyphMask) {
        if self.map.len() >= self.cap && !self.map.contains_key(&key) {
            // Linear scan: find the key with the smallest tick.
            if let Some((&evict_key, _)) = self.map.iter().min_by_key(|(_, (_, t))| t) {
                self.map.remove(&evict_key);
            }
        }
        self.tick += 1;
        self.map.insert(key, (mask, self.tick));
    }
}
use std::ptr::NonNull;

use anvil_render::{FontMetrics, GlyphPainter, PixelRect};
use objc2_core_foundation::{
    CFArray, CFArrayCallBacks, CFAttributedString, CFBoolean, CFDictionary,
    CFDictionaryKeyCallBacks, CFDictionaryValueCallBacks, CFError, CFRetained, CFString, CGPoint,
    CGSize,
};
#[allow(deprecated)]
use objc2_core_text::{
    CTFont, CTFontDescriptor, CTFontManagerRegisterGraphicsFont, CTFontOrientation,
    CTFontSymbolicTraits, CTLine, kCTFontAttributeName, kCTFontFeatureSettingsAttribute,
    kCTForegroundColorFromContextAttributeName,
};
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

/// Which face variant a `Font` represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontFace {
    Regular,
    Bold,
    Italic,
    BoldItalic,
    /// Fixed 11 pt (× scale) chrome face — used for tab bar, status bar, etc.
    Chrome,
}

impl FontFace {
    fn symbolic_traits(self) -> CTFontSymbolicTraits {
        match self {
            FontFace::Bold => CTFontSymbolicTraits::TraitBold,
            FontFace::Italic => CTFontSymbolicTraits::TraitItalic,
            FontFace::BoldItalic => CTFontSymbolicTraits(
                CTFontSymbolicTraits::TraitBold.0 | CTFontSymbolicTraits::TraitItalic.0,
            ),
            FontFace::Regular | FontFace::Chrome => CTFontSymbolicTraits(0),
        }
    }

    fn needs_traits(self) -> bool {
        !matches!(self, FontFace::Regular | FontFace::Chrome)
    }
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

        Font::from_ct(ct)
    }

    /// Create a font for the given face and pixel size.
    ///
    /// For `Bold`, `Italic`, and `BoldItalic` faces the Regular font is loaded
    /// first, then `copy_with_symbolic_traits` requests the matching variant.
    /// If CoreText cannot find a bold/italic face in the family (the Nerd Font
    /// build may not ship them), the method falls back to affine-skew synthesis
    /// for italic and weight-shift is logged as unavailable.
    ///
    /// When `ligatures` is true the returned font is wrapped with an OpenType
    /// feature descriptor that enables `calt` (contextual alternates).  This
    /// should be false for the chrome face to avoid surprises in the Basin mark.
    pub fn init_face(
        names: &[&str],
        pixel_size: f64,
        face: FontFace,
        ligatures: bool,
    ) -> Result<Font, FontError> {
        // Load the base Regular font first from the fallback chain.
        let base = Font::init_first_available(names, pixel_size)?;

        let ct: CFRetained<CTFont> = if face.needs_traits() {
            let traits = face.symbolic_traits();
            // SAFETY: size=0.0 preserves original size; null matrix = identity.
            let variant = unsafe {
                base.ct
                    .copy_with_symbolic_traits(0.0, std::ptr::null(), traits, traits)
            };
            match variant {
                Some(v) => v,
                None => {
                    // Fallback: for italic faces apply an affine shear matrix.
                    // For bold-only, log and use the regular face — synthesized
                    // bold (double draw, weight filter) is out of scope.
                    let needs_italic = face == FontFace::Italic || face == FontFace::BoldItalic;
                    if needs_italic {
                        eprintln!(
                            "anvil: italic face unavailable for \"{}\"; synthesizing via shear",
                            names.first().unwrap_or(&"?")
                        );
                        // 12° shear: c = tan(12°) ≈ 0.2126.
                        let skew = objc2_core_foundation::CGAffineTransform {
                            a: 1.0,
                            b: 0.0,
                            c: 0.2126,
                            d: 1.0,
                            tx: 0.0,
                            ty: 0.0,
                        };
                        // SAFETY: skew is a valid stack CGAffineTransform.
                        unsafe { base.ct.copy_with_attributes(0.0, &skew as *const _, None) }
                    } else {
                        eprintln!(
                            "anvil: bold face unavailable for \"{}\"; using regular",
                            names.first().unwrap_or(&"?")
                        );
                        base.ct
                    }
                }
            }
        } else {
            base.ct
        };

        // Optionally enable the calt (contextual alternates) OpenType feature.
        let ct = if ligatures { enable_calt(ct) } else { ct };

        Font::from_ct(ct)
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

    /// Build a `Font` from an already-loaded `CTFont`.
    fn from_ct(ct: CFRetained<CTFont>) -> Result<Font, FontError> {
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
}

/// Wrap `ct` in a `CTFontDescriptor` that enables the `calt` OpenType
/// feature (contextual alternates — responsible for `->`, `=>`, `!=` etc.
/// in IBM Plex Mono).
///
/// Uses the macOS 10.10+ simplified form: a CFArray containing a single
/// CFString `"calt"` is sufficient as a `kCTFontFeatureSettingsAttribute`
/// value.  Shaping (rendering ligatures per-run) is a separate follow-up;
/// this call ensures the feature is active on the CTFont descriptor so that
/// if the render path ever makes full ligature shaping calls, the feature is
/// already requested.
fn enable_calt(base: CFRetained<CTFont>) -> CFRetained<CTFont> {
    // Build: CFArray [ CFString("calt") ]
    let tag_str = CFString::from_str("calt");
    let tag_ptr = tag_str.as_ref() as *const CFString as *const c_void;
    let mut values = [tag_ptr];

    // SAFETY: values is a valid 1-element *const c_void array; callbacks
    // pointer points to the global kCFTypeArrayCallBacks constant.
    let array: Option<CFRetained<CFArray>> = unsafe {
        unsafe extern "C" {
            static kCFTypeArrayCallBacks: CFArrayCallBacks;
        }
        CFArray::new(
            None,
            values.as_mut_ptr(),
            1,
            &kCFTypeArrayCallBacks as *const CFArrayCallBacks,
        )
    };
    let array = match array {
        Some(a) => a,
        None => {
            eprintln!("anvil: calt CFArray creation failed; ligature flag not set");
            return base;
        }
    };

    // Build: CFDictionary { kCTFontFeatureSettingsAttribute => array }
    #[allow(clippy::borrow_deref_ref)]
    let key_ptr = unsafe { &*kCTFontFeatureSettingsAttribute as *const CFString as *const c_void };
    let val_ptr = array.as_ref() as *const CFArray as *const c_void;
    let mut keys = [key_ptr];
    let mut vals = [val_ptr];

    // SAFETY: keys/vals are 1-element arrays; callback pointers are global statics.
    let dict: Option<CFRetained<CFDictionary>> = unsafe {
        unsafe extern "C" {
            static kCFTypeDictionaryKeyCallBacks: CFDictionaryKeyCallBacks;
            static kCFTypeDictionaryValueCallBacks: CFDictionaryValueCallBacks;
        }
        CFDictionary::new(
            None,
            keys.as_mut_ptr(),
            vals.as_mut_ptr(),
            1,
            &kCFTypeDictionaryKeyCallBacks as *const CFDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks as *const CFDictionaryValueCallBacks,
        )
    };
    let dict = match dict {
        Some(d) => d,
        None => {
            eprintln!("anvil: calt CFDictionary creation failed; ligature flag not set");
            return base;
        }
    };

    // Create a CTFontDescriptor from the attributes dict, then copy the
    // base CTFont with that descriptor applied.
    // SAFETY: dict is a valid CFDictionary of CTFont attribute key-value pairs.
    let desc = unsafe { CTFontDescriptor::with_attributes(&dict) };

    // SAFETY: size=0.0 preserves size; null matrix = identity; desc is valid.
    unsafe { base.copy_with_attributes(0.0, std::ptr::null(), Some(&desc)) }
}

/// Group of five `Font` instances covering all faces used in one app session.
///
/// `grid[0..4]` = Regular, Bold, Italic, BoldItalic (in `FontFace` order).
/// `chrome` = 11 pt × scale face used for tab bar, status bar, etc.
pub struct FontBundle {
    /// `[Regular, Bold, Italic, BoldItalic]` — same family, same pixel size.
    pub grid: [Font; 4],
    /// Smaller fixed-size font for chrome UI elements (tab bar, status bar).
    pub chrome: Font,
}

impl FontBundle {
    /// Load all five faces.  `names` is the terminal font fallback chain;
    /// `pixel_size` is the terminal size (pt × scale).  Chrome is always
    /// `CHROME_PT * scale` regardless of `pixel_size`.
    pub fn new(names: &[&str], pixel_size: f64, scale: f64) -> Result<FontBundle, FontError> {
        let regular = Font::init_face(names, pixel_size, FontFace::Regular, true)?;
        let bold = Font::init_face(names, pixel_size, FontFace::Bold, true)?;
        let italic = Font::init_face(names, pixel_size, FontFace::Italic, true)?;
        let bold_italic = Font::init_face(names, pixel_size, FontFace::BoldItalic, true)?;

        // Assert that all four grid faces share the same advance width —
        // required for monospace alignment.
        let rw = regular.metrics.cell_w;
        for (f, w) in [
            ("bold", bold.metrics.cell_w),
            ("italic", italic.metrics.cell_w),
            ("bold_italic", bold_italic.metrics.cell_w),
        ] {
            if (w - rw).abs() > 1.0 {
                eprintln!(
                    "anvil: {f} face advance {w:.1} != regular {rw:.1}; monospace alignment may break"
                );
            }
        }

        let chrome_px = CHROME_PT * scale;
        let chrome = Font::init_face(names, chrome_px, FontFace::Chrome, false)?;

        Ok(FontBundle {
            grid: [regular, bold, italic, bold_italic],
            chrome,
        })
    }

    /// Return the grid `Font` for the given bold/italic combination.
    ///
    /// Index mapping: `[Regular=0, Bold=1, Italic=2, BoldItalic=3]`.
    pub fn face(&self, bold: bool, italic: bool) -> &Font {
        let idx = match (bold, italic) {
            (false, false) => 0,
            (true, false) => 1,
            (false, true) => 2,
            (true, true) => 3,
        };
        &self.grid[idx]
    }

    /// Pre-rasterize printable ASCII (U+0020–U+007E) for all four grid faces
    /// and return the warmed painters.
    ///
    /// 4 faces × 95 glyphs = 380 cache entries — well within the 2048-entry
    /// LRU cap.  The chrome face is intentionally excluded (few distinct glyphs).
    ///
    /// Returns `[Regular, Bold, Italic, BoldItalic]` painters with their
    /// caches pre-filled so the first rendered frame skips CoreText for ASCII.
    pub fn warm_ascii_atlas(&self) -> [CoreTextPainter<'_>; 4] {
        std::array::from_fn(|i| {
            let mut painter = CoreTextPainter::new(&self.grid[i]);
            painter.warm_ascii();
            painter
        })
    }
}

/// Chrome font size in logical points (rendered at CHROME_PT × scale device px).
pub const CHROME_PT: f64 = 11.0;

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
    cache: GlyphCache,
}

impl<'a> CoreTextPainter<'a> {
    pub fn new(font: &'a Font) -> Self {
        Self {
            font,
            rasterizer: None,
            cache: GlyphCache::new(GLYPH_CACHE_CAP),
        }
    }

    /// Pre-rasterize printable ASCII (U+0020–U+007E) into the glyph cache.
    ///
    /// Uses the font's own cell metrics to build the rasterizer, then inserts
    /// each glyph mask without compositing into a pixel buffer.  Should be
    /// called once after construction so that the first rendered frame hits
    /// only cache lookups for common ASCII glyphs.
    ///
    /// Silent in release builds (no output).
    pub fn warm_ascii(&mut self) {
        let metrics = self.font.metrics;
        let cell_w = metrics.cell_w.round() as usize;
        let cell_h = metrics.cell_h.round() as usize;
        if cell_w == 0 || cell_h == 0 {
            return;
        }
        // Build the rasterizer once for this cell size.
        self.rasterizer = Rasterizer::new(cell_w, cell_h, metrics.descent);
        if self.rasterizer.is_none() {
            return;
        }
        for cp in 0x20u32..=0x7E {
            let glyph = self.font.glyph(cp);
            if glyph == 0 {
                continue;
            }
            if self.cache.get(glyph).is_none() {
                let mask = self
                    .rasterizer
                    .as_mut()
                    .unwrap()
                    .rasterize(&self.font.ct, glyph);
                self.cache.insert(glyph, mask);
            }
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

        // Rasterize the glyph once; subsequent calls hit the LRU cache.
        if self.cache.get(glyph).is_none() {
            let mask = self
                .rasterizer
                .as_mut()
                .unwrap()
                .rasterize(&self.font.ct, glyph);
            self.cache.insert(glyph, mask);
        }
        let mask = self.cache.get(glyph).unwrap();

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

impl CoreTextPainter<'_> {
    /// Draw a shaped run of codepoints into a BGRA8 pixel buffer.
    ///
    /// Unlike `draw_glyph`, which draws one codepoint per call (preventing
    /// ligature substitution), `draw_run` gathers all codepoints into a single
    /// `CFAttributedString`, shapes them through `CTLine::with_attributed_string`,
    /// and draws the resulting glyph run in one pass. This enables OpenType
    /// `calt` ligature substitution (e.g. `->` → `→`, `!=` → `≠`) when the
    /// font has the feature active (see `Font::init_face` with `ligatures: true`).
    ///
    /// # Arguments
    ///
    /// * `codepoints` — Unicode scalar values for consecutive same-style cells.
    ///   Must be non-empty; all cells must share fg/bg/attrs (the caller is
    ///   responsible for splitting at style boundaries).
    /// * `start_x` / `dest_y` — top-left of the first cell, in top-down bitmap
    ///   pixels (same coordinate space as `PixelRect` in `draw_glyph`).
    /// * `fg` — RGB foreground color used to tint the shaped glyphs.
    /// * `pixels` / `bitmap_width` / `bitmap_height` — the BGRA8 destination.
    /// * `cell_w` / `cell_h` / `descent` — per-font metrics (from `Font::metrics`).
    ///
    /// # Why the caller must opt in
    ///
    /// The existing `draw_glyph` render path in `draw.rs` calls this painter
    /// once per cell. Upgrading it to produce same-style runs requires the
    /// render loop to accumulate runs before calling the painter — a change to
    /// `crates/anvil-render/src/draw.rs` that is deferred to a future render
    /// rewrite. This method is provided now so that infrastructure is ready
    /// when the render side is updated.
    ///
    /// # Limitations (TODO for the render rewrite)
    ///
    /// * The `draw_glyph` path does **not** call this method. Per-cell rendering
    ///   prevents visible ligature substitution regardless of the `calt` flag on
    ///   the CTFont descriptor.
    /// * Wide (double-width) glyphs produced by shaping are not accounted for —
    ///   the run width is fixed at `cell_w * codepoints.len()`. The render loop
    ///   will need to query the shaped advance and reconcile it with the grid.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_run(
        &self,
        codepoints: &[u32],
        start_x: f64,
        dest_y: f64,
        fg: [u8; 3],
        pixels: &mut [u8],
        bitmap_width: usize,
        bitmap_height: usize,
        cell_w: f64,
        cell_h: f64,
        descent: f64,
    ) {
        if codepoints.is_empty() {
            return;
        }

        // ── 1. Build a UTF-16 CFString from the codepoints ───────────────────
        let mut utf16: Vec<u16> = Vec::with_capacity(codepoints.len() * 2);
        for &cp in codepoints {
            if cp <= 0xFFFF {
                utf16.push(cp as u16);
            } else {
                let v = cp - 0x10000;
                utf16.push(0xD800 + (v >> 10) as u16);
                utf16.push(0xDC00 + (v & 0x3FF) as u16);
            }
        }
        // SAFETY: utf16 is a valid non-null u16 slice.
        let cf_str = unsafe {
            unsafe extern "C-unwind" {
                fn CFStringCreateWithCharacters(
                    alloc: *const c_void,
                    chars: *const u16,
                    num_chars: objc2_core_foundation::CFIndex,
                ) -> Option<std::ptr::NonNull<CFString>>;
            }
            let raw = CFStringCreateWithCharacters(
                std::ptr::null(),
                utf16.as_ptr(),
                utf16.len() as objc2_core_foundation::CFIndex,
            );
            match raw {
                Some(p) => CFRetained::from_raw(p),
                None => return,
            }
        };

        // ── 2. Build attributes dict:
        //      { kCTFontAttributeName: self.font.ct,
        //        kCTForegroundColorFromContextAttributeName: kCFBooleanTrue }
        //
        // Setting kCTForegroundColorFromContextAttributeName tells CTLine::draw
        // to use the CG context's current fill color (white) for text, instead
        // of requiring a CGColor in kCTForegroundColorAttributeName.
        #[allow(clippy::borrow_deref_ref)]
        let font_key = unsafe { &*kCTFontAttributeName as *const CFString as *const c_void };
        let font_val = self.font.ct.as_ref() as *const CTFont as *const c_void;
        // SAFETY: kCFBooleanTrue is a valid static CF object.
        #[allow(clippy::borrow_deref_ref)]
        let fg_key = unsafe {
            &*kCTForegroundColorFromContextAttributeName as *const CFString as *const c_void
        };
        let fg_val: *const c_void = unsafe {
            unsafe extern "C" {
                static kCFBooleanTrue: Option<&'static CFBoolean>;
            }
            match kCFBooleanTrue {
                Some(b) => b as *const CFBoolean as *const c_void,
                None => return,
            }
        };
        let mut keys = [font_key, fg_key];
        let mut vals = [font_val, fg_val];
        // SAFETY: 2-element key/value arrays with valid CF callback statics.
        let attr_dict: CFRetained<CFDictionary> = unsafe {
            unsafe extern "C" {
                static kCFTypeDictionaryKeyCallBacks: CFDictionaryKeyCallBacks;
                static kCFTypeDictionaryValueCallBacks: CFDictionaryValueCallBacks;
            }
            match CFDictionary::new(
                None,
                keys.as_mut_ptr(),
                vals.as_mut_ptr(),
                2,
                &kCFTypeDictionaryKeyCallBacks as *const CFDictionaryKeyCallBacks,
                &kCFTypeDictionaryValueCallBacks as *const CFDictionaryValueCallBacks,
            ) {
                Some(d) => d,
                None => return,
            }
        };

        // ── 3. Create CFAttributedString ─────────────────────────────────────
        // SAFETY: cf_str and attr_dict are valid retained CF objects.
        let attr_str = unsafe {
            CFAttributedString::new(None, Some(cf_str.as_ref()), Some(attr_dict.as_ref()))
        };
        let attr_str = match attr_str {
            Some(s) => s,
            None => return,
        };

        // ── 4. Shape: CTLine applies calt and returns the glyph run ──────────
        // SAFETY: attr_str is a valid CFAttributedString.
        let line = unsafe { CTLine::with_attributed_string(&attr_str) };

        // ── 5. Rasterize the shaped line into a wide RGBA buffer ─────────────
        //
        // CTLine::draw requires a full-color context (it uses the CoreText text
        // rendering pipeline, unlike the lower-level CTFont::draw_glyphs which
        // works in a gray context). We allocate a 4-bytes-per-pixel RGBA buffer,
        // draw with white fill, then extract the red channel as a coverage mask.
        let run_cells = codepoints.len();
        let run_w = (cell_w * run_cells as f64).ceil() as usize;
        let run_h = cell_h.ceil() as usize;
        if run_w == 0 || run_h == 0 {
            return;
        }
        let stride = run_w * 4;
        let mut run_buf = vec![0u8; stride * run_h];
        let srgb = match objc2_core_graphics::CGColorSpace::new_device_rgb() {
            Some(cs) => cs,
            None => return,
        };
        // kCGImageAlphaPremultipliedLast = 1; RGBA, 8 bpc, premultiplied alpha.
        let ctx = unsafe {
            objc2_core_graphics::CGBitmapContextCreate(
                run_buf.as_mut_ptr() as *mut c_void,
                run_w,
                run_h,
                8,
                stride,
                Some(&srgb),
                1, // kCGImageAlphaPremultipliedLast
            )
        };
        let ctx = match ctx {
            Some(c) => c,
            None => return,
        };
        // Text matrix identity + white fill so R channel = coverage.
        objc2_core_graphics::CGContext::set_text_matrix(Some(&ctx), unsafe {
            objc2_core_graphics::CGAffineTransformIdentity
        });
        objc2_core_graphics::CGContext::set_rgb_fill_color(Some(&ctx), 1.0, 1.0, 1.0, 1.0);
        // Baseline in CG y-up = descent above the bottom of the context.
        objc2_core_graphics::CGContext::set_text_position(Some(&ctx), 0.0, descent);

        // SAFETY: ctx is a valid CGContext; line is a valid CTLine.
        unsafe { line.draw(&ctx) }
        drop(ctx);
        drop(srgb);

        // ── 6. Extract red channel as coverage mask, composite into dest ─────
        // run_buf is RGBA premultiplied; for white-on-black text the red channel
        // == coverage. Build a GlyphMask with that single channel.
        let mut coverage = vec![0u8; run_w * run_h];
        for (i, chunk) in run_buf.chunks_exact(4).enumerate() {
            coverage[i] = chunk[0]; // R of premultiplied RGBA = coverage
        }
        let run_mask = GlyphMask {
            pixels: coverage,
            width: run_w,
            height: run_h,
        };
        composite_mask(
            &run_mask,
            start_x,
            dest_y,
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

    /// brand mono font stack loads with sane metrics
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

    /// glyph lookup resolves common characters
    #[test]
    fn glyph_lookup_resolves_common_characters() {
        let names = &["IBMPlexMono", "SFMono-Regular", "Menlo"];
        let f = Font::init_first_available(names, 26.0).unwrap();
        assert!(f.glyph('A' as u32) != 0);
        assert!(f.glyph('z' as u32) != 0);
        assert!(f.glyph('0' as u32) != 0);
    }

    /// glyph handles an astral-plane codepoint via the surrogate-pair path
    #[test]
    fn glyph_handles_astral_plane_codepoint() {
        let names = &["IBMPlexMono", "SFMono-Regular", "Menlo"];
        let f = Font::init_first_available(names, 26.0).unwrap();
        // U+1F600 is above the BMP; the lookup must not crash.
        let _ = f.glyph(0x1F600);
    }

    /// initFirstAvailable with no names returns NoFontAvailable
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

    /// FontFace::Bold symbolic traits include TraitBold.
    #[test]
    fn font_face_bold_has_correct_traits() {
        let traits = FontFace::Bold.symbolic_traits();
        assert_eq!(
            traits.0 & CTFontSymbolicTraits::TraitBold.0,
            CTFontSymbolicTraits::TraitBold.0
        );
        assert_eq!(traits.0 & CTFontSymbolicTraits::TraitItalic.0, 0);
    }

    /// FontFace::Italic symbolic traits include TraitItalic.
    #[test]
    fn font_face_italic_has_correct_traits() {
        let traits = FontFace::Italic.symbolic_traits();
        assert_eq!(
            traits.0 & CTFontSymbolicTraits::TraitItalic.0,
            CTFontSymbolicTraits::TraitItalic.0
        );
        assert_eq!(traits.0 & CTFontSymbolicTraits::TraitBold.0, 0);
    }

    /// FontFace::BoldItalic symbolic traits include both Bold and Italic.
    #[test]
    fn font_face_bold_italic_has_both_traits() {
        let traits = FontFace::BoldItalic.symbolic_traits();
        assert_eq!(
            traits.0 & CTFontSymbolicTraits::TraitBold.0,
            CTFontSymbolicTraits::TraitBold.0
        );
        assert_eq!(
            traits.0 & CTFontSymbolicTraits::TraitItalic.0,
            CTFontSymbolicTraits::TraitItalic.0
        );
    }

    /// init_face Regular with ligatures loads with sane metrics.
    #[test]
    fn init_face_regular_with_ligatures_loads() {
        let names = &["IBMPlexMono", "SFMono-Regular", "Menlo"];
        let f = Font::init_face(names, 26.0, FontFace::Regular, true).unwrap();
        assert!(f.metrics.cell_w > 0.0);
        assert!(f.metrics.cell_h > 0.0);
    }

    /// init_face Bold does not crash and returns a font with sane metrics.
    #[test]
    fn init_face_bold_loads() {
        let names = &["IBMPlexMono", "SFMono-Regular", "Menlo"];
        let f = Font::init_face(names, 26.0, FontFace::Bold, true).unwrap();
        assert!(f.metrics.cell_w > 0.0);
        assert!(f.metrics.cell_h > 0.0);
    }

    /// init_face Italic does not crash and returns a font with sane metrics.
    #[test]
    fn init_face_italic_loads() {
        let names = &["IBMPlexMono", "SFMono-Regular", "Menlo"];
        let f = Font::init_face(names, 26.0, FontFace::Italic, true).unwrap();
        assert!(f.metrics.cell_w > 0.0);
        assert!(f.metrics.cell_h > 0.0);
    }

    /// FontBundle::new loads all five faces without panicking.
    #[test]
    fn font_bundle_new_loads_all_faces() {
        let names = &["IBMPlexMono", "SFMono-Regular", "Menlo"];
        let bundle = FontBundle::new(names, 26.0, 2.0).unwrap();
        assert!(bundle.grid[0].metrics.cell_w > 0.0); // Regular
        assert!(bundle.grid[1].metrics.cell_w > 0.0); // Bold
        assert!(bundle.grid[2].metrics.cell_w > 0.0); // Italic
        assert!(bundle.grid[3].metrics.cell_w > 0.0); // BoldItalic
        // Chrome is smaller: CHROME_PT=11 * scale=2 = 22px < 26px.
        assert!(bundle.chrome.metrics.cell_w < bundle.grid[0].metrics.cell_w + 1.0);
    }

    /// Chrome font metrics are visibly smaller than the terminal font.
    #[test]
    fn chrome_font_is_smaller_than_terminal_font() {
        let names = &["IBMPlexMono", "SFMono-Regular", "Menlo"];
        // Terminal at 30px = 15pt × 2× scale; chrome at 11pt × 2× = 22px.
        let bundle = FontBundle::new(names, 30.0, 2.0).unwrap();
        assert!(
            bundle.chrome.metrics.cell_h < bundle.grid[0].metrics.cell_h,
            "chrome cell_h {} must be < terminal cell_h {}",
            bundle.chrome.metrics.cell_h,
            bundle.grid[0].metrics.cell_h
        );
    }

    /// `draw_run` shapes a multi-codepoint run and inks pixels.
    ///
    /// Verifies the shaping infrastructure is wired up end-to-end: a
    /// CFAttributedString is built, CTLine shapes it, and the result composites
    /// into the destination buffer.
    #[test]
    fn draw_run_inks_pixels_for_a_run_of_codepoints() {
        let names = &["IBMPlexMono", "SFMono-Regular", "Menlo"];
        let font = Font::init_face(names, 26.0, FontFace::Regular, true).unwrap();
        let painter = CoreTextPainter::new(&font);
        let cell_w = font.metrics.cell_w.ceil();
        let cell_h = font.metrics.cell_h.ceil();
        let descent = font.metrics.descent;
        let run = &['A' as u32, 'B' as u32, 'C' as u32];
        let bmp_w = (cell_w * run.len() as f64) as usize;
        let bmp_h = cell_h as usize;
        let mut pixels = vec![0u8; bmp_w * bmp_h * 4];
        painter.draw_run(
            run,
            0.0,
            0.0,
            [255, 255, 255],
            &mut pixels,
            bmp_w,
            bmp_h,
            cell_w,
            cell_h,
            descent,
        );
        let inked = pixels.iter().filter(|&&b| b != 0).count();
        assert!(inked > 0, "draw_run inked no pixels for 'A', 'B', 'C'");
    }
}
