//! Headless chrome snapshot tool.
//!
//! Renders Anvil chrome (tab bar, context bar, left dock, content placeholder)
//! to a PNG without launching the AppKit window or PTY.
//!
//! Usage:
//!   cargo run --bin anvil-snapshot -- \
//!       --width 1700 --height 1100 \
//!       --theme ember-dark \
//!       --cwd /home/user/project \
//!       --out /tmp/render.png

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

use anvil_render::agent_panel::LocalContext;
use anvil_render::context_bar::draw_context_bar;
use anvil_render::raster::{FontMetrics, GlyphPainter, PixelRect, Raster, UiTextPainter, UiWeight};
use anvil_render::tabbar::{TabBarHits, draw_tab_bar};
use anvil_theme::Theme;
use anvil_workspace::tab::{Tab, TabManager};

// ── Stub painters ─────────────────────────────────────────────────────────────

/// No-op glyph painter — we have no font at snapshot time.
struct NullGlyphPainter;

impl GlyphPainter for NullGlyphPainter {
    fn draw_glyph(
        &mut self,
        _codepoint: u32,
        _dest: PixelRect,
        _fg: [u8; 3],
        _metrics: FontMetrics,
        _pixels: &mut [u8],
        _bitmap_width: usize,
        _bitmap_height: usize,
    ) {
    }
}

/// No-op UI text painter — returns zero width, paints nothing.
struct NullUiPainter;

impl UiTextPainter for NullUiPainter {
    fn measure(&mut self, _text: &str, _size_pt: f64, _weight: UiWeight) -> f64 {
        0.0
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_line(
        &mut self,
        _text: &str,
        _x_px: f64,
        _baseline_y_px: f64,
        _size_pt: f64,
        _weight: UiWeight,
        _fg: [u8; 3],
        _pixels: &mut [u8],
        _bitmap_w: usize,
        _bitmap_h: usize,
    ) {
    }
}

// ── CLI argument parsing ───────────────────────────────────────────────────────

struct Args {
    width: usize,
    height: usize,
    theme: String,
    cwd: String,
    out: PathBuf,
}

fn parse_args() -> Args {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut width = 1700usize;
    let mut height = 1100usize;
    let mut theme = "ember-dark".to_string();
    let mut cwd = "/tmp/project".to_string();
    let mut out = PathBuf::from("/tmp/anvil-snapshot.png");

    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "--width" => {
                if let Some(v) = raw.get(i + 1) {
                    width = v.parse().unwrap_or(width);
                    i += 1;
                }
            }
            "--height" => {
                if let Some(v) = raw.get(i + 1) {
                    height = v.parse().unwrap_or(height);
                    i += 1;
                }
            }
            "--theme" => {
                if let Some(v) = raw.get(i + 1) {
                    theme = v.clone();
                    i += 1;
                }
            }
            "--cwd" => {
                if let Some(v) = raw.get(i + 1) {
                    cwd = v.clone();
                    i += 1;
                }
            }
            "--out" => {
                if let Some(v) = raw.get(i + 1) {
                    out = PathBuf::from(v);
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    Args {
        width,
        height,
        theme,
        cwd,
        out,
    }
}

// ── PNG writer (BGRA → RGBA) ───────────────────────────────────────────────────

fn write_png(path: &PathBuf, pixels: &[u8], width: usize, height: usize) -> std::io::Result<()> {
    let file = File::create(path)?;
    let w = BufWriter::new(file);
    let mut encoder = png::Encoder::new(w, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    // Convert BGRA → RGBA.
    let mut rgba = vec![0u8; pixels.len()];
    for i in (0..pixels.len()).step_by(4) {
        rgba[i] = pixels[i + 2]; // R
        rgba[i + 1] = pixels[i + 1]; // G
        rgba[i + 2] = pixels[i]; // B
        rgba[i + 3] = pixels[i + 3]; // A
    }

    writer
        .write_image_data(&rgba)
        .map_err(|e| std::io::Error::other(e.to_string()))
}

// ── Render scene ──────────────────────────────────────────────────────────────

fn render(args: &Args) -> Raster {
    let w = args.width;
    let h = args.height;

    let theme: Theme = anvil_theme::by_name(&args.theme);
    let mut raster = Raster::new(w, h);
    let mut glyph = NullGlyphPainter;
    let mut ui = NullUiPainter;

    // Metrics: approximate cell size for a 13pt mono font at 2× scale.
    let metrics = FontMetrics {
        cell_w: 8.0,
        cell_h: 18.0,
        descent: 4.0,
    };

    // Chrome heights (device pixels, matching main.rs geometry).
    let window_scale = 2.0f64;
    let chrome_pt = 36.0f64; // CHROME_PT from anvil-platform
    let chrome_px = chrome_pt * window_scale;
    let context_bar_h = metrics.cell_h + 12.0;

    // Fill background.
    raster.clear(theme.background);

    // ── Tab bar ────────────────────────────────────────────────────────────────
    let mut tab_mgr = TabManager::default();
    let tab = Tab::new_single_pane(80, 24, 0);
    tab_mgr.push(tab);
    let mut tab_hits = TabBarHits::default();
    draw_tab_bar(
        &mut raster,
        &mut glyph,
        &mut ui,
        metrics,
        &theme,
        &tab_mgr,
        "main",  // branch
        "12:00", // clock
        window_scale,
        chrome_px,
        &mut tab_hits,
        0.0, // tab_strip_scroll
    );

    // ── Context bar (IDE identity bar) ────────────────────────────────────────
    let local = LocalContext {
        cwd: args.cwd.clone(),
        ..Default::default()
    };
    let bar_rect = anvil_workspace::layout::Rect {
        x: 0.0,
        y: chrome_px,
        w: w as f64,
        h: context_bar_h,
    };
    draw_context_bar(
        &mut raster,
        &mut glyph,
        &mut ui,
        metrics,
        &theme,
        &local,
        None, // no editor context
        bar_rect,
    );

    // ── Left dock placeholder ─────────────────────────────────────────────────
    let dock_w = 300.0f64;
    let content_top = chrome_px + context_bar_h;
    raster.fill_pixel_rect(
        0.0,
        content_top,
        dock_w,
        (h as f64) - content_top,
        theme.charcoal,
    );
    // Hairline divider.
    raster.fill_pixel_rect(
        dock_w,
        content_top,
        1.0,
        (h as f64) - content_top,
        theme.hairline,
    );

    // ── Content area placeholder ───────────────────────────────────────────────
    raster.fill_pixel_rect(
        dock_w + 1.0,
        content_top,
        (w as f64) - dock_w - 1.0,
        (h as f64) - content_top,
        theme.surface,
    );

    raster
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    let args = parse_args();
    let raster = render(&args);
    match write_png(&args.out, raster.bytes(), raster.width, raster.height) {
        Ok(()) => {
            println!(
                "wrote {} ({}×{})",
                args.out.display(),
                raster.width,
                raster.height
            );
        }
        Err(e) => {
            eprintln!("anvil-snapshot: write failed: {e}");
            std::process::exit(1);
        }
    }
}
