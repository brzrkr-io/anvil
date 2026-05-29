//! Headless chrome snapshot tool.
//!
//! Renders Anvil chrome (tab bar, context bar, left dock, content placeholder)
//! to a PNG without launching the AppKit window or PTY.
//!
//! Usage:
//!   cargo run --bin anvil-snapshot -- \
//!       --width 1700 --height 1100 \
//!       --theme mineral-dark \
//!       --cwd /home/user/project \
//!       --out /tmp/render.png

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

use anvil_config::UiFontCfg;
use anvil_platform::UiPainter;
use anvil_render::agent_panel::LocalContext;
use anvil_render::context_bar::draw_context_bar;
use anvil_render::left_dock::{DirEntry, DirSnapshot, OutlineKind, OutlineRow, draw_left_dock};
use anvil_render::raster::{FontMetrics, GlyphPainter, PixelRect, Raster};
use anvil_render::tabbar::{TabBarHits, draw_tab_bar};
use anvil_theme::Theme;
use anvil_workspace::tab::{Tab, TabManager};

// ── Stub painters ─────────────────────────────────────────────────────────────

/// No-op glyph painter — mono glyphs (basin mark, file icons) are out of scope
/// for the headless snapshot; UI text is rendered via the real UiPainter.
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

// ── CLI argument parsing ───────────────────────────────────────────────────────

/// Named scene — selects which Anvil UI state to render.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scene {
    /// IDE mode, no file open, welcome card placeholder.
    Welcome,
    /// IDE mode, Rust file open with outline symbols populated.
    CodeOpen,
    /// IDE mode + command-palette overlay (Cmd+P).
    PaletteOpen,
    /// IDE mode + bottom shell drawer with simulated output.
    DrawerShell,
    /// Terminal-only mode, full-height terminal placeholder.
    TerminalOnly,
}

impl Scene {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "welcome" => Some(Self::Welcome),
            "code-open" => Some(Self::CodeOpen),
            "palette-open" => Some(Self::PaletteOpen),
            "drawer-shell" => Some(Self::DrawerShell),
            "terminal-only" => Some(Self::TerminalOnly),
            _ => None,
        }
    }
}

struct Args {
    width: usize,
    height: usize,
    theme: String,
    cwd: String,
    out: PathBuf,
    /// Optional scene override. When `None` the legacy single-frame render runs.
    scene: Option<Scene>,
}

fn parse_args() -> Args {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut width = 1700usize;
    let mut height = 1100usize;
    let mut theme = "mineral-dark".to_string();
    let mut cwd = "/tmp/project".to_string();
    let mut out = PathBuf::from("/tmp/anvil-snapshot.png");
    let mut scene: Option<Scene> = None;

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
            "--scene" => {
                if let Some(v) = raw.get(i + 1) {
                    scene = Scene::from_name(v.as_str());
                    if scene.is_none() {
                        eprintln!(
                            "anvil-snapshot: unknown scene '{v}'. \
                             Valid: welcome, code-open, palette-open, drawer-shell, terminal-only"
                        );
                        std::process::exit(1);
                    }
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
        scene,
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

/// Geometry constants shared across all scenes.
struct SceneGeometry {
    w: f64,
    h: f64,
    window_scale: f64,
    chrome_px: f64,
    context_bar_h: f64,
    content_top: f64,
    dock_w: f64,
    metrics: FontMetrics,
}

impl SceneGeometry {
    fn new(w: usize, h: usize) -> Self {
        let metrics = FontMetrics {
            cell_w: 8.0,
            cell_h: 18.0,
            descent: 4.0,
        };
        let window_scale = 2.0f64;
        let chrome_pt = 36.0f64;
        let chrome_px = chrome_pt * window_scale;
        let context_bar_h = metrics.cell_h + 12.0;
        let content_top = chrome_px + context_bar_h;
        let dock_w = 300.0f64;
        Self {
            w: w as f64,
            h: h as f64,
            window_scale,
            chrome_px,
            context_bar_h,
            content_top,
            dock_w,
            metrics,
        }
    }
}

/// Draw the chrome rows (tab bar + context bar) common to all IDE scenes.
fn draw_ide_chrome(
    raster: &mut Raster,
    glyph: &mut dyn GlyphPainter,
    ui: &mut UiPainter,
    geo: &SceneGeometry,
    theme: &Theme,
    cwd: &str,
    tab_mgr: &TabManager,
) {
    let mut tab_hits = TabBarHits::default();
    draw_tab_bar(
        raster,
        glyph,
        ui,
        geo.metrics,
        theme,
        tab_mgr,
        "main",
        "12:00",
        geo.window_scale,
        geo.chrome_px,
        &mut tab_hits,
        0.0,
    );
    let local = LocalContext {
        cwd: cwd.to_string(),
        ..Default::default()
    };
    let bar_rect = anvil_workspace::layout::Rect {
        x: 0.0,
        y: geo.chrome_px,
        w: geo.w,
        h: geo.context_bar_h,
    };
    draw_context_bar(
        raster,
        glyph,
        ui,
        geo.metrics,
        theme,
        &local,
        None,
        bar_rect,
    );
}

fn render_welcome(
    raster: &mut Raster,
    glyph: &mut dyn GlyphPainter,
    ui: &mut UiPainter,
    geo: &SceneGeometry,
    theme: &Theme,
    cwd: &str,
) {
    raster.clear(theme.background);
    let mut tab_mgr = TabManager::default();
    tab_mgr.push(Tab::new_single_pane(80, 24, 0));
    draw_ide_chrome(raster, glyph, ui, geo, theme, cwd, &tab_mgr);

    // Left dock — empty cwd.
    let dock_rect = anvil_workspace::layout::Rect {
        x: 0.0,
        y: geo.content_top,
        w: geo.dock_w,
        h: geo.h - geo.content_top,
    };
    draw_left_dock(
        raster,
        glyph,
        ui,
        geo.metrics,
        theme,
        None,
        None,
        None,
        dock_rect,
    );

    // Content: welcome card placeholder — a centred panel block.
    let cx = geo.dock_w + 1.0;
    let cw = geo.w - cx;
    raster.fill_pixel_rect(
        cx,
        geo.content_top,
        cw,
        geo.h - geo.content_top,
        theme.surface,
    );
    // Welcome card: centred filled rect in panel color.
    let card_w = (cw * 0.6).min(600.0);
    let card_h = 160.0;
    let card_x = cx + (cw - card_w) * 0.5;
    let card_y = geo.content_top + (geo.h - geo.content_top - card_h) * 0.4;
    raster.fill_pixel_rect(card_x, card_y, card_w, card_h, theme.panel);
}

fn render_code_open(
    raster: &mut Raster,
    glyph: &mut dyn GlyphPainter,
    ui: &mut UiPainter,
    geo: &SceneGeometry,
    theme: &Theme,
    cwd: &str,
) {
    raster.clear(theme.background);
    let mut tab_mgr = TabManager::default();
    tab_mgr.push(Tab::new_single_pane(80, 24, 0));
    draw_ide_chrome(raster, glyph, ui, geo, theme, cwd, &tab_mgr);

    let snap = DirSnapshot {
        root: cwd.to_string(),
        entries: vec![
            DirEntry {
                name: "src".to_string(),
                is_dir: true,
                is_symlink: false,
            },
            DirEntry {
                name: "Cargo.toml".to_string(),
                is_dir: false,
                is_symlink: false,
            },
        ],
        git_marks: Default::default(),
    };
    let outline_rows = vec![
        OutlineRow {
            name: "main".to_string(),
            kind: OutlineKind::Function,
            depth: 0,
            line: 1,
        },
        OutlineRow {
            name: "App".to_string(),
            kind: OutlineKind::Struct,
            depth: 0,
            line: 10,
        },
        OutlineRow {
            name: "render".to_string(),
            kind: OutlineKind::Function,
            depth: 1,
            line: 20,
        },
    ];
    let dock_rect = anvil_workspace::layout::Rect {
        x: 0.0,
        y: geo.content_top,
        w: geo.dock_w,
        h: geo.h - geo.content_top,
    };
    draw_left_dock(
        raster,
        glyph,
        ui,
        geo.metrics,
        theme,
        Some(&snap),
        None,
        Some(&outline_rows),
        dock_rect,
    );
    // Editor area: simulated code lines.
    let cx = geo.dock_w + 1.0;
    let cw = geo.w - cx;
    raster.fill_pixel_rect(
        cx,
        geo.content_top,
        cw,
        geo.h - geo.content_top,
        theme.surface,
    );
    // Gutter stripe.
    raster.fill_pixel_rect(
        cx,
        geo.content_top,
        40.0,
        geo.h - geo.content_top,
        theme.charcoal,
    );
}

fn render_palette_open(
    raster: &mut Raster,
    glyph: &mut dyn GlyphPainter,
    ui: &mut UiPainter,
    geo: &SceneGeometry,
    theme: &Theme,
    cwd: &str,
) {
    // Start with the welcome scene as the base.
    render_welcome(raster, glyph, ui, geo, theme, cwd);
    // Overlay: command palette — a centred panel rectangle with hairline border.
    let overlay_w = (geo.w * 0.5).min(700.0);
    let overlay_h = 320.0;
    let overlay_x = (geo.w - overlay_w) * 0.5;
    let overlay_y = geo.content_top + 20.0;
    raster.fill_pixel_rect(overlay_x, overlay_y, overlay_w, overlay_h, theme.panel);
    // Hairline border.
    raster.fill_pixel_rect(overlay_x, overlay_y, overlay_w, 1.0, theme.hairline);
    raster.fill_pixel_rect(
        overlay_x,
        overlay_y + overlay_h - 1.0,
        overlay_w,
        1.0,
        theme.hairline,
    );
    raster.fill_pixel_rect(overlay_x, overlay_y, 1.0, overlay_h, theme.hairline);
    raster.fill_pixel_rect(
        overlay_x + overlay_w - 1.0,
        overlay_y,
        1.0,
        overlay_h,
        theme.hairline,
    );
    // Input row at top of overlay.
    raster.fill_pixel_rect(
        overlay_x + 1.0,
        overlay_y + 1.0,
        overlay_w - 2.0,
        40.0,
        theme.charcoal,
    );
    // Accent stripe under input.
    raster.fill_pixel_rect(
        overlay_x + 1.0,
        overlay_y + 40.0,
        overlay_w - 2.0,
        2.0,
        theme.accent_primary,
    );
}

fn render_drawer_shell(
    raster: &mut Raster,
    glyph: &mut dyn GlyphPainter,
    ui: &mut UiPainter,
    geo: &SceneGeometry,
    theme: &Theme,
    cwd: &str,
) {
    raster.clear(theme.background);
    let mut tab_mgr = TabManager::default();
    tab_mgr.push(Tab::new_single_pane(80, 24, 0));
    draw_ide_chrome(raster, glyph, ui, geo, theme, cwd, &tab_mgr);

    let snap = DirSnapshot {
        root: cwd.to_string(),
        entries: vec![DirEntry {
            name: "src".to_string(),
            is_dir: true,
            is_symlink: false,
        }],
        git_marks: Default::default(),
    };
    let dock_rect = anvil_workspace::layout::Rect {
        x: 0.0,
        y: geo.content_top,
        w: geo.dock_w,
        h: geo.h - geo.content_top,
    };
    draw_left_dock(
        raster,
        glyph,
        ui,
        geo.metrics,
        theme,
        Some(&snap),
        None,
        None,
        dock_rect,
    );

    // Editor area (72% of content height).
    let cx = geo.dock_w + 1.0;
    let cw = geo.w - cx;
    let content_h = geo.h - geo.content_top;
    let editor_h = (content_h * 0.72).floor();
    let drawer_h = content_h - editor_h;
    raster.fill_pixel_rect(cx, geo.content_top, cw, editor_h, theme.surface);
    raster.fill_pixel_rect(cx, geo.content_top, 40.0, editor_h, theme.charcoal);
    // Divider.
    raster.fill_pixel_rect(cx, geo.content_top + editor_h, cw, 1.0, theme.hairline);
    // Drawer: terminal-colored background.
    raster.fill_pixel_rect(
        cx,
        geo.content_top + editor_h + 1.0,
        cw,
        drawer_h - 1.0,
        theme.background,
    );
}

fn render_terminal_only(
    raster: &mut Raster,
    glyph: &mut dyn GlyphPainter,
    ui: &mut UiPainter,
    geo: &SceneGeometry,
    theme: &Theme,
    cwd: &str,
) {
    raster.clear(theme.background);
    // Terminal mode: tab bar only (no context bar, no dock).
    let mut tab_mgr = TabManager::default();
    tab_mgr.push(Tab::new_single_pane(80, 24, 0));
    let mut tab_hits = TabBarHits::default();
    draw_tab_bar(
        raster,
        glyph,
        ui,
        geo.metrics,
        theme,
        &tab_mgr,
        "main",
        "12:00",
        geo.window_scale,
        geo.chrome_px,
        &mut tab_hits,
        0.0,
    );
    let _ = cwd; // Terminal mode shows no cwd bar.
    // Full-width terminal content area.
    let term_y = geo.chrome_px;
    raster.fill_pixel_rect(0.0, term_y, geo.w, geo.h - term_y, theme.background);
}

fn render(args: &Args) -> Raster {
    let w = args.width;
    let h = args.height;

    let theme: Theme = anvil_theme::by_name(&args.theme);
    let mut raster = Raster::new(w, h);
    let mut glyph = NullGlyphPainter;
    let mut ui = UiPainter::new(UiFontCfg::default(), 2.0);
    let geo = SceneGeometry::new(w, h);

    match args.scene {
        Some(Scene::Welcome) => {
            render_welcome(&mut raster, &mut glyph, &mut ui, &geo, &theme, &args.cwd)
        }
        Some(Scene::CodeOpen) => {
            render_code_open(&mut raster, &mut glyph, &mut ui, &geo, &theme, &args.cwd)
        }
        Some(Scene::PaletteOpen) => {
            render_palette_open(&mut raster, &mut glyph, &mut ui, &geo, &theme, &args.cwd)
        }
        Some(Scene::DrawerShell) => {
            render_drawer_shell(&mut raster, &mut glyph, &mut ui, &geo, &theme, &args.cwd)
        }
        Some(Scene::TerminalOnly) => {
            render_terminal_only(&mut raster, &mut glyph, &mut ui, &geo, &theme, &args.cwd)
        }
        None => render_legacy(&mut raster, &mut glyph, &mut ui, &geo, &theme, &args.cwd),
    }

    raster
}

/// Original single-frame render (legacy path, no --scene flag).
fn render_legacy(
    raster: &mut Raster,
    glyph: &mut dyn GlyphPainter,
    ui: &mut UiPainter,
    geo: &SceneGeometry,
    theme: &Theme,
    cwd: &str,
) {
    let w = geo.w;
    let h = geo.h;

    // CC5: use real CoreText-backed painter so chrome text is visible in snapshots.
    // Metrics: approximate cell size for a 13pt mono font at 2× scale.
    let metrics = geo.metrics;

    // Fill background.
    raster.clear(theme.background);

    // ── Tab bar ────────────────────────────────────────────────────────────────
    let mut tab_mgr = TabManager::default();
    let tab = Tab::new_single_pane(80, 24, 0);
    tab_mgr.push(tab);
    let mut tab_hits = TabBarHits::default();
    draw_tab_bar(
        raster,
        glyph,
        ui,
        metrics,
        theme,
        &tab_mgr,
        "main",  // branch
        "12:00", // clock
        geo.window_scale,
        geo.chrome_px,
        &mut tab_hits,
        0.0, // tab_strip_scroll
    );

    // ── Context bar (IDE identity bar) ────────────────────────────────────────
    let local = LocalContext {
        cwd: cwd.to_string(),
        ..Default::default()
    };
    let bar_rect = anvil_workspace::layout::Rect {
        x: 0.0,
        y: geo.chrome_px,
        w,
        h: geo.context_bar_h,
    };
    draw_context_bar(
        raster, glyph, ui, metrics, theme, &local, None, // no editor context
        bar_rect,
    );

    // ── Left dock placeholder ─────────────────────────────────────────────────
    let dock_w = geo.dock_w;
    let content_top = geo.content_top;
    raster.fill_pixel_rect(0.0, content_top, dock_w, h - content_top, theme.charcoal);
    // Hairline divider.
    raster.fill_pixel_rect(dock_w, content_top, 1.0, h - content_top, theme.hairline);

    // ── Content area placeholder ───────────────────────────────────────────────
    raster.fill_pixel_rect(
        dock_w + 1.0,
        content_top,
        w - dock_w - 1.0,
        h - content_top,
        theme.surface,
    );
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
