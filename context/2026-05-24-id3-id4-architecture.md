# ID3 + ID4 — Left Dock (Explorer + Outline) and Right Dock (Run Panel)

Date: 2026-05-24. Track C, ID3 + ID4. Builds on ID1 geometry. Scope:
paint `areas.left_dock` and `areas.right_dock` in Ide mode. No editor
surface, no LSP, no resize handle.

## 1. Codebase confirmations

- **cwd source.** `App::current_cwd()` (`crates/anvil/src/main.rs:553`)
  returns the focused pane's OSC 7 path. ID3 reads it each frame; no
  new App-side plumbing.
- **Right dock is already AG1.** `draw_right_hud`
  (`agent_panel.rs:573`) paints the docked HUD column. Pixel math at
  `main.rs:1687` (right-anchored slab `hud_cols*cw + GRID_PAD`) already
  matches `Docks::for_mode(Ide,…).right_w`.
- **One-line ID4 bug.** `eff_hud = self.hud_visible` (`main.rs:1453`)
  must become `self.hud_visible || self.layout_mode == LayoutMode::Ide`.
  That's the entire behavioural change for ID4. No new render module.

## 2. New modules

- `crates/anvil-render/src/left_dock.rs` — `draw_left_dock`, splits
  the dock 60/40 (Explorer / Outline), owns section headers + divider
  hairline. Private: `draw_explorer_section`, `draw_outline_section`.
- `crates/anvil/src/fs_worker.rs` — background filesystem walker
  mirroring `crates/anvil/src/kube.rs` (named thread,
  `mpsc::SyncSender`, debounced cwd input).

No state added to `agent_panel.rs` or `anvil-workspace`. Tree model
lives on `App`.

## 3. ID3 data model

```rust
// fs_worker.rs
pub struct DirSnapshot { pub root: PathBuf, pub entries: Vec<Entry> }
pub struct Entry { pub name: String, pub is_dir: bool }  // dirs first, ci-sorted

// App fields
fs_tx: SyncSender<PathBuf>,
fs_rx: Receiver<DirSnapshot>,
fs_cache: HashMap<PathBuf, Vec<Entry>>,
fs_expanded: HashSet<PathBuf>,         // root auto-expanded
fs_selected: Option<PathBuf>,
left_dock_hits: Vec<LeftDockHit>,
```

`LeftDockHit { rect: PixelRect, path: PathBuf, is_dir: bool }`.
Immediate-children-only with lazy expansion: keeps model O(visible),
avoids `node_modules`-class blowups.

## 4. ID3 threading

One thread `anvil-fs`, mirroring `anvil-kube`:

1. Main thread on cwd-change / expand-toggle: `fs_tx.try_send(path)`.
2. Worker drains pending (cap most-recent 8), `std::fs::read_dir` each,
   sends `DirSnapshot` back.
3. 2 s per-path debounce; repeats coalesce. Event-driven only.
4. Main thread per frame: `while let Ok(s) = fs_rx.try_recv() {
   cache.insert(...) }`; redraw on change.

Skip-list filtered in the worker (same as `recent_files_in_dir`):
`.git`, `node_modules`, `target`, `.DS_Store`, leading-dot entries.
Symlinks: one-hop max, no loop detection in v1.

Failure modes:
- `read_dir` EACCES/ENOENT → empty snapshot → `(unreadable)`/`(empty)`.
- Worker panic → stale cache; log once. Not restarted (kube pattern).
- cwd empty → Waiting empty-state.
- Channel full → drop; next event retries.

## 5. ID4 — Run panel wiring

Single predicate flip. Keep existing AG1 pixel math at `main.rs:1687`
— it already matches `areas.right_dock.w`. Add a debug-build assert
`surface_rect.w == areas.right_dock.w` to catch drift. Same
`draw_right_hud` covers Terminal Cmd+\\ and Ide-permanent paths.

### Geometry
- Left dock: 260 px @ 1× (ID1 default).
- Right dock: `HUD_COLS*cell_w + cell_w` ≈ 240 px @ 1× (AG1 value).
  Asymmetry intentional — left holds indent + filename; right holds
  wider AG1 section content. Do not retune in ID3/ID4.

## 6. ID3 rendering

`draw_left_dock(raster, painter, metrics, theme, rect, fs_cache,
fs_expanded, fs_selected, cwd, hits)`.

- Section header strip: 22 px, `theme.charcoal`, hairline at bottom.
  Label in `text_muted` caps: `EXPLORER`, `OUTLINE`.
- Tree row: 20 px, indent 12 px per depth. `▸` collapsed / `▾`
  expanded for dirs; nothing for files. Drawn via existing
  `raster.glyph_at`.
- Selection: row fill `theme.surface`, 2 px left stripe in
  `theme.accent_primary` (matches active-tab convention).
- Long names: `…` ellipsis at right.
- Vertical overflow: clipped to `rect.h`. **No scrolling in v1.**

Outline section: header + empty-state lines only.

## 7. Empty-state copy (exact)

- No cwd: `Waiting for shell prompt…` (text_muted, centred).
- Dir empty: `(empty)` (indented one level, text_muted).
- Dir unreadable: `(unreadable)` (same treatment).
- Outline always v1: line 1 `Outline unavailable`, line 2
  `Requires nvim bridge (BR5)`. text_muted, left-padded.

No fake symbols. No "Coming soon".

## 8. Interaction (mouse)

Hit-test only when `layout_mode == Ide` and click inside
`areas.left_dock`.

- Click dir → toggle `fs_expanded`; if uncached, `fs_tx.try_send`.
- Click file → set `fs_selected`. **Visual no-op otherwise.**
- No double-click, keyboard nav, drag, or context menu in v1.

`left_dock_hits` cleared and refilled per frame (same as
`tab_bar_hits`).

## 9. Theme tokens

`background` (canvas), `charcoal` (section header), `hairline` (header
bottom + section divider), `surface` (selected row fill), `foreground`
(names + `▾`), `text_muted` (header labels, empty-state, `▸`),
`accent_primary` (2 px selected-row stripe). No new tokens.

## 10. Explicit deferrals

Selection → editor; git badges; extension icons; drag/rename/new/
delete; Cmd+click reveal-in-Finder; keyboard nav; scrolling; Outline
content (BR5); resizable divider; per-tab cache (tab switch re-
requests; `fs_expanded` is global); symlink-loop detection; FSEvents
watch (we're event-driven on cwd-change / expand only).

## 11. Verification plan

Unit (`anvil-render`):
- `draw_left_dock` no-op when `rect.w == 0`.
- Empty cwd → Waiting line painted.
- Cached entries empty → `(empty)` row painted.
- Hits cleared+refilled each call.

Unit (`anvil/fs_worker`):
- Tempdir 3 files + 1 dir → sorted `Entry` list, dirs first.
- Unreadable path → empty `Vec`, no panic.

Manual:
- `ANVIL_LAYOUT_MODE=ide`: left dock shows cwd basename expanded with
  immediate children; AG1 HUD on right without Cmd+\\.
- Click dir → children appear within one frame.
- `cd ~/elsewhere` → tree re-roots within 2 s.
- Cmd+Shift+E cycle: Terminal → Ide reveals dock; Ide → Codex hides
  both; Codex → Terminal restores HUD toggle.
- `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`
  green.

## 12. Open questions

1. **Explorer/Outline split.** Proposing 60/40. Outline empty in v1 so
   only cosmetic — keep 60/40, or collapse Outline to a header row
   until BR5?
2. **Selection persistence.** Session-only proposed. Config key worth
   it?
3. **Hide dotfiles by default?** Proposing yes, no toggle in v1.
4. **Per-tab cwd memory.** Tab switch re-requests; `fs_expanded` is
   global. Acceptable?

## 13. Proposed `wiki/decisions/` entry

`wiki/decisions/2026-05-24-left-dock-fs-walker.md` — durable choice:
"Explorer reads focused-pane OSC 7 cwd, walks one dir level at a time
on a dedicated `anvil-fs` thread (kube pattern), caches per-path entry
lists, uses lazy expansion. No FSEvents in v1." Captures why: avoid
O(N) trees, never block the render thread, keep worker pattern uniform
with `anvil-kube`.

Build order:
1. `fs_worker.rs` + App fields + cwd-change wiring — verify: tempdir
   unit test green; worker logs reads.
2. Flip `eff_hud` to include Ide mode (ID4 done) — verify:
   `ANVIL_LAYOUT_MODE=ide` paints HUD without Cmd+\; Terminal mode
   unchanged.
3. `left_dock.rs` skeleton + section headers + empty-state copy —
   verify: dock paints, empty cwd shows Waiting.
4. Tree render + indent + glyphs + clipping — verify: basename and
   children visible; selection rendered.
5. Mouse hit-test + expand/collapse + `fs_tx` requests — verify:
   clicking a dir lists children within one frame.
— verify after each.
