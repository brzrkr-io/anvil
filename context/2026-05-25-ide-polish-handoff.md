---
date: 2026-05-25
kind: handoff
status: live
goal: continue IDE polish (Hermes items 7-20) after the items 1-6 slice
---

# Handoff — IDE Polish, items 7–20

## Where the branch is

- Branch: `rust-port`, ahead of `origin/rust-port` by **2 commits** (unpushed).
- Working tree: **clean**.
- All gates green at HEAD:
  - `cargo fmt --all`
  - `cargo clippy --workspace -- -D warnings`
  - `cargo test --workspace`

```
ed290d6 feat(ide): polish Explorer rows + bottom drawer for operator-console feel
5bcddab feat(ide): explorer mouse hit-targets, row selection, wheel scroll
b523f9b feat(editor): rescue native IDE surface and theme modes  (previous baseline)
```

## What just landed (Hermes items 1–6)

1. **Explorer mouse slice (`5bcddab`).** 32px full-width row hit targets,
   header+row click routing through a single `explorer_path_for_hit` helper,
   wheel-over-dock scroll without stealing scroll from the focused pane.
2. **Visual polish slice (`ed290d6`).** Per the design spec at
   `context/2026-05-25-ide-polish-slice-decisions.md`:
   - Explorer rows shrunk to **22px** with **10px** padding; selected row uses
     solid `panel` + `accent_primary` left rail + `foreground` text; alt-row
     stripe removed; hover paints solid `panel` (suppressed when selected).
   - Bottom drawer: charcoal wash replaced with solid `panel`; top rule always
     `hairline`. Empty-pane branch now paints `panel` + 22px `charcoal` header
     strip with a `text_subtle` `"TERMINAL  ⌘T"` label instead of a raw
     `background` rect.

## Known carry-over from items 1–6

- **Passive Explorer hover is half-wired.** The render path takes
  `hovered_row: Option<usize>` and paints correctly. The input path only
  updates the field from `mouse_dragged` (button held) because
  `AppHandler` has no `mouse_moved`. **Adding `fn mouse_moved` to
  `crates/anvil-platform/src/appkit.rs` (with NSTrackingArea or
  `acceptsMouseMovedEvents`) is required to make hover actually feel
  like hover.** This is a 1–2 file `anvil-platform` change + a wiring
  line in `main.rs`. Likely the right thing to slot in before tackling
  item 5 again.

## Recommended next slice (items 7–10)

Tight, related, similar surface area to what just landed:

- **Item 5 finish — passive hover.** Add `mouse_moved` to AppHandler +
  NSTrackingArea on the window content view in `anvil-platform`. Wire
  through `main.rs` `mouse_moved` → update `hovered_explorer_row` and
  also clear it when cursor exits the dock. **Do this first** — it
  unblocks visible hover and makes items 7/8 testable.
- **Item 7 — directory click.** Clicking a folder in Explorer currently
  routes through `open_path_in_native_editor` and re-snapshots the cwd.
  Replace with expand-in-place (preferred — matches VS Code/Zed) or
  drill-in with a breadcrumb. If drill-in: needs a back affordance in
  the Explorer header.
- **Item 8 — scroll affordance.** Wheel scroll works (`5bcddab` proved
  it) but there is no visible scrollbar/overflow cue. Add a thin
  right-edge scroll indicator that fades when not scrolling.
- **Item 9 — overflow smoke fixture.** No automated proof that wheel
  scroll moves the visible rows. Add a test that drives many entries
  through the explorer (the test in `5bcddab` proves indices preserve;
  this should prove that rendered rows shift). Optional: a
  `scripts/run.sh`-driven smoke in a temp dir with N entries.
- **Item 10 — Outline.** Currently a "No symbols yet / Open a source
  file" placeholder. Two options: (a) hide when empty / collapse to a
  quiet header, (b) wire a real symbol source. (a) is cheap and matches
  the "quiet secondary panel" direction; (b) needs an LSP symbol
  request through the editor crate. **Suggest (a) for this slice**, (b)
  as a later editor-side task.

## Backlog (items 11–20, untouched)

From Hermes's original list, in his priority order:

- 11. Tighten editor chrome (file chip, header, spacing, active top accent).
- 12. README. truncation bug (basename should preserve extension).
- 13. Editor typography hierarchy (content readable; chrome quieter).
- 14. Real tab / open-buffer UI for the native editor.
- 15. Re-smoke Cmd+E idempotency under current changes (screenshot).
- 16. Multi-file open smoke (README → Cargo.toml, same editor surface reuses).
- 17. Save/dirty indicator polish in tab/chrome/status line.
- 18. Right-side context panel strategy decision (hidden by default? summon?).
- 19. Visual diff vs. accepted companion render; close biggest deltas.
- 20. Full closeout pass (fmt/clippy/test + screenshot suite).

Items 11–14 are likely the next coherent visual slice after 7–10. Item 18
is a **design decision** that should go to `design-lead` before any code.

## Routing recommendation for the next agent

1. **builder** — add `mouse_moved` to `anvil-platform` (item 5 finish);
   small, well-scoped, unlocks hover testability.
2. **design-lead** — quick spec for items 7, 8, 10 (directory click
   model, scroll affordance token + width, Outline empty-state). Should
   be short — most decisions inherit from the items 2–6 spec already
   in `context/2026-05-25-ide-polish-slice-decisions.md`.
3. **builder** — execute items 7, 8, 10 against that spec.
4. **reviewer** — closeout pass + gate check before the next handoff.

Per AGENTS.md orchestrator-first rule, the next session should still
dispatch `orchestrator` first to confirm or adjust this routing — this
handoff is the input, not the plan.

## Useful pointers

- Design spec for the slice just landed:
  `context/2026-05-25-ide-polish-slice-decisions.md`
- Authoritative IDE redesign target: `context/2026-05-24-ide-redesign.md`
- Brand: `BRAND.md` (mineral palette, status semantics).
- Theme tokens: `crates/anvil-theme/src/theme.rs` (MINERAL_DARK,
  MINERAL_LIGHT).
- Files most likely to change next: `crates/anvil-platform/src/appkit.rs`,
  `crates/anvil-render/src/left_dock.rs`, `crates/anvil/src/main.rs`.

## Open questions for the user / next session

- Push `rust-port` upstream now, or wait until items 7–10 also land?
  (Branch is 2 commits ahead.)
- Items 7 directory click: expand-in-place vs. drill-in? Default
  recommendation: **expand-in-place**; matches modern IDEs and avoids
  adding breadcrumb/back chrome to the Explorer header.
- Item 18 right-side context panel: keep hidden, summon on demand, or
  redesign? Needs a product/design call before code.
