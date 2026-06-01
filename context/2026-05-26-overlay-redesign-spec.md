---
title: Overlay Redesign Spec
date: 2026-05-26
status: ready-for-builder
owner: systems-architect
---

# Overlay Redesign — Builder Spec

Replace the ~17 ad-hoc overlay draws in `crates/anvil/src/main.rs` with a unified, animated card-overlay subsystem in `crates/anvil-render`. Hybrid architecture (option 3): three widget primitives + an `Overlay` enum, with a custom-render escape hatch for outliers.

## 1. Module Layout

New module tree under `crates/anvil-render/src/overlay/`:

- `mod.rs` — `OverlayId`, `Overlay` enum, `OverlayStack`, `OverlayRenderCtx`, public API.
- `chrome.rs` — `draw_card_chrome` (background, border, shadow, padding).
- `anim.rs` — `OverlayAnim` (scale + alpha state machine, easing).
- `text.rs` — `OverlayText` seam: `draw_label`, `draw_mono`. Today both call the monospace glyph painter; swap to proportional in one place when track A lands.
- `widgets/picker.rs` — `PickerOverlay` (query input + scrollable list).
- `widgets/text_input.rs` — `TextInputOverlay` (prompt + single-line editor).
- `widgets/tooltip.rs` — `TooltipOverlay` (anchored, non-modal, click-through).
- `input.rs` — `OverlayInputRouter`: maps `KeyEvent` / mouse to top-of-stack.

Re-export from `crates/anvil-render/src/lib.rs` as `pub mod overlay`.

App-side: add `App.overlays: OverlayStack` in `crates/anvil/src/main.rs`; the old `*Picker`, `*Search`, `*Popup` fields collapse into stack entries.

## 2. Core Types

```rust
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum OverlayId {
    FilePicker, WorkspaceSymbols, BufferSymbols, ProjectSearch,
    ScmPanel, BranchSwitcher, GitLog, ThemePicker, LangPicker,
    Completion, CodeActions, LspReferences, LspRename,
    GotoLine, SaveAs, OpenFolder, ProjectSwitcher,
    Hover, BlameTip, FileTip,
}

pub enum Overlay {
    Picker(PickerOverlay),
    TextInput(TextInputOverlay),
    Tooltip(TooltipOverlay),
    Custom(Box<dyn CustomOverlay>),
}

pub trait CustomOverlay {
    fn id(&self) -> OverlayId;
    fn measure(&self, ctx: &OverlayMeasureCtx) -> CardSize;
    fn render(&self, ctx: &mut OverlayRenderCtx);
    fn handle_key(&mut self, ev: &KeyEvent) -> OverlayKeyResult;
    fn handle_click(&mut self, loc: MouseLocation) -> OverlayClickResult;
    fn close_on_blur(&self) -> bool { true }
}

pub struct OverlayStack { entries: Vec<OverlayEntry> }
struct OverlayEntry { overlay: Overlay, anim: OverlayAnim, opened_at: Instant }

pub enum OverlayKeyResult { Consumed, Close, Submit(Submission), PassThrough }
pub enum OverlayClickResult { Consumed, Close, PassThrough }
```

Only top entry receives input; render iterates bottom-up so stacked overlays compose.

## 3. Card Chrome (`chrome.rs`)

```rust
pub struct CardGeom {
    pub x: f64, pub y: f64, pub w: f64, pub h: f64,
    pub radius: f64,        // 8.0 * ui_scale; 0.0 if no rounded fill
    pub padding: f64,       // 16.0 * ui_scale
    pub anim_scale: f64,    // 0.96..1.00
    pub anim_alpha: f64,    // 0..1
}

pub fn draw_card_chrome(raster: &mut Raster, theme: &Theme, geom: CardGeom, show_scrim: bool);
```

Order: scrim (`#000` α 0.28 × anim_alpha) → 3 shadow rects (offsets +2/+4/+8, α 0.10/0.06/0.03 × anim_alpha) → panel fill (`theme.panel` × anim_alpha) → 1px top inner highlight (`theme.surface` α 0.3) → 1px border (`theme.hairline`). Scale applied around card center.

Rounded corners v1: square + `// TODO(radius)` and a `raster::fill_rounded_rect` stub. Phase 4 flips on.

## 4. Animation (`anim.rs`)

```rust
pub struct OverlayAnim { pub state: AnimState, pub t: f64, pub duration_ms: f64 }
impl OverlayAnim {
    pub fn tick(&mut self, dt_ms: f64) -> bool;
    pub fn scale(&self) -> f64;  // ease_out_cubic(t)*0.04 + 0.96
    pub fn alpha(&self) -> f64;
    pub fn begin_close(&mut self);
    pub fn finished(&self) -> bool;
}
```

`AppShell` calls `overlays.tick(dt)` per frame; if any animating, request redraw.

## 5. Input Router (`input.rs`)

`OverlayInputRouter::dispatch_key(stack, ev)`:
1. Empty stack → `PassThrough`.
2. Top not `Visible` → consume but ignore.
3. `Esc` → `top.begin_close()`, return `Consumed`.
4. Else delegate to overlay; `Close`/`Submit` triggers `begin_close()` + emits submission.

Picker keys handled inside `PickerOverlay::handle_key`: ↑/↓ moves selection, Enter submits, Tab/Shift+Tab cycles filter chips, printable chars append to query, Backspace deletes. `TextInputOverlay` handles printable + arrow + home/end + enter.

Mouse: `dispatch_click(stack, loc)` → top; `Close` from outside-click only if `close_on_blur()`.

## 6. Widget Primitives

```rust
pub struct PickerRow { pub primary: String, pub secondary: Option<String>, pub badge: Option<Badge> }
pub struct PickerOverlay {
    pub id: OverlayId,
    pub title: Option<String>,
    pub query: String,
    pub rows: Vec<PickerRow>,
    pub selected: usize,
    pub max_visible: usize,
    pub on_submit: PickerCallback,  // typed enum
}
```

`TextInputOverlay { id, prompt, value, cursor, on_submit }`.
`TooltipOverlay { id, anchor: Anchor, body: TooltipBody, follow_cursor: bool }` where `Anchor::Pixel(x,y)` or `Anchor::EditorCell(row,col)`.

Layout: chrome → padding → header → body. Picker body = visible window around `selected`, each row 1.4× line_height, two-tone. All text via `OverlayText::draw_label`.

## 7. Text Seam (`text.rs`)

```rust
pub trait OverlayPainter {
    fn label(&mut self, x: f64, y: f64, s: &str, color: [u8;3], weight: Weight);
    fn measure(&self, s: &str, weight: Weight) -> f64;
}
pub struct MonoPainter<'a> { /* wraps current GlyphPainter */ }
pub struct UiPainter<'a>  { /* future proportional */ }
```

`OverlayRenderCtx::text() -> &mut dyn OverlayPainter` returns `MonoPainter` today. Track A swaps to `UiPainter` for non-code labels — single change, all widgets follow.

## 8. Migration Table

| # | Overlay | Current fn | Target |
|---|---|---|---|
| 1 | File picker (Cmd+P) | webview, `send_file_picker_show` | `Picker` (ref) |
| 2 | Project search | `draw_project_search_overlay` | `Picker` |
| 3 | Goto line | `draw_goto_line_overlay` | `TextInput` |
| 4 | LSP rename | `draw_lsp_rename_overlay` | `TextInput` |
| 5 | Save as | `draw_save_as_overlay` | `TextInput` |
| 6 | Open folder | `draw_open_folder_overlay` | `TextInput` |
| 7 | Theme picker | `draw_theme_picker_overlay` | `Picker` |
| 8 | Language picker | `draw_language_picker_overlay` | `Picker` |
| 9 | LSP references | `draw_lsp_references_overlay` | `Picker` |
| 10 | Workspace symbols | `draw_workspace_symbol_overlay` | `Picker` |
| 11 | Buffer symbols | `draw_buffer_symbol_overlay` | `Picker` |
| 12 | Project switcher | `draw_project_switcher_overlay` | `Picker` |
| 13 | SCM panel | `draw_scm_panel_overlay` | `Custom` |
| 14 | Branch switcher | `draw_branch_switcher_overlay` | `Picker` |
| 15 | Git log | `draw_git_log_overlay` | `Picker` |
| 16 | Completion popup | `editor.rs:775` | `Custom` |
| 17 | Code actions | `editor.rs:849` | `Picker` |
| 18 | Hover popup | `editor.rs:956` | `Custom` |
| 19 | Blame tooltip | inline | `Tooltip` |
| 20 | File tooltip | inline | `Tooltip` |

Retire `draw_overlay_scrim_and_shadow` + `draw_overlay_shadow` once last caller migrates.

## 9. Phases

- **Phase 1 — Foundation (1d).** Module skeleton, `OverlayStack`, `OverlayAnim`, `draw_card_chrome`, `OverlayInputRouter`, text seam.
- **Phase 2 — Reference (0.5d).** Migrate ProjectSearch. Feature-flag `--overlay-v2` for one commit, then flip.
- **Phase 3 — Migration (1.5–2d).** Groups: TextInputs → Pickers → Tooltips → Custom. Delete each old draw_*_overlay as caller flips.
- **Phase 4 — Polish (0.5d).** Add `fill_rounded_rect`, enable 8px radius. Tune anim. Verify dark + light themes.

## 10. Tests

Per-primitive:
- `picker_filters_rows_on_query`, `picker_arrows_clamp`, `text_input_enter_emits_submission`, `tooltip_anchor_positions_card`, `anim_alpha_zero_at_entering_start_one_at_visible`, `chrome_paints_shadow_then_panel_then_border`.

Per-overlay smoke (in `crates/anvil/src/overlay_smoke.rs`): push to stack, tick to Visible, send key, assert state.

## 11. Assumptions / Non-Goals

- No real backdrop blur (Metal compositing — separate track).
- No rounded corners until `raster::fill_rounded_rect` ships.
- Proportional font deferred to track A; mono seam covers v1.
- Webview Cmd+P file picker stays as-is for v1.
- `Overlay` variants own state; no `dyn Fn` closures (keeps types `Send`).

## 12. Failure Modes

- Stuck anim → `AppShell` redraws whenever `stack.animating()`.
- Input-after-close race → router treats `Leaving` entries as transparent to input.
- Stack leak → `OverlayStack::gc` drops `Leaving && finished`.
- Theme mismatch → chrome reads `panel`/`hairline`; verify both themes define them in test.
