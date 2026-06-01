---
date: 2026-05-25
kind: handoff
status: live
goal: close visual gap to "render A" target (Hermes items 11-20)
---

# Handoff — IDE Polish, items 11–20

## Where the branch is

- Branch: `rust-port`, ahead of `origin/rust-port` by **7 commits** (unpushed).
- Working tree: **clean**.
- All gates green at HEAD:
  - `cargo fmt --all`
  - `cargo clippy --workspace -- -D warnings`
  - `cargo test --workspace`

```
94bb6f3 fix(ide): chevron uses text_subtle per spec, not text_muted
2c342d2 docs(context): design spec for IDE polish items 7, 8, 10
b8b0188 feat(ide): items 7-10 expand-in-place + scroll affordance + outline empty-state
e36b327 feat(ide): passive Explorer hover via NSTrackingArea + mouse_moved
c40b7ef docs(context): handoff for items 7-20 IDE polish work
ed290d6 feat(ide): polish Explorer rows + bottom drawer for operator-console feel
5bcddab feat(ide): explorer mouse hit-targets, row selection, wheel scroll
```

## What just landed (items 5, 7–10)

- **Item 5 (`e36b327`)** — Passive Explorer hover: `mouse_moved` +
  `mouse_exited` + `updateTrackingAreas` on `AnvilView`; `NSTrackingArea`
  options `ActiveInKeyWindow | InVisibleRect | MouseMoved |
  MouseEnteredAndExited`. Wired into `AppShell::mouse_moved` and cleared
  on exit. Manual verification still pending — agents could not launch
  the GUI.
- **Items 7–10 (`b8b0188`)** —
  - Item 7: `expanded_dirs: HashSet<usize>` on `App`, cleared on snapshot
    change. Chevron glyph swaps `▸`/`▾`. Directory click no longer
    re-snapshots cwd; only toggles the set (stub expansion; real
    nested-child rendering is a follow-on task).
  - Item 8: 3px right-edge scroll thumb in `text_subtle * (alpha * 0.6)`;
    hidden when content fits. Per-frame decay: 600ms hold + 200ms ease-out.
    Fade-in is currently instant (defensible reading of spec; flagged below).
  - Item 9: `overflow_scroll_changes_rendered_entries` test proves wheel
    scroll shifts the visible row set, not just the offset value.
  - Item 10: Outline empty-state collapses to a single 22px header row in
    `text_subtle`; body copy removed; divider preserved; header reverts
    to `accent_bright` when symbols arrive.
- **Reviewer fix (`94bb6f3`)** — Chevron color changed `text_muted` →
  `text_subtle` to match spec.
- **Spec doc (`2c342d2`)** — `context/2026-05-25-ide-polish-items-7-8-10-spec.md`.

## Known carry-over from this slice

1. **Manual UI verification pending for items 5, 7, 8, 10.** All code paths
   compile and pass the smoke test; nobody has launched `scripts/run.sh`
   and watched a cursor enter the Explorer. The next session should do
   this first — it takes 30 seconds and validates the whole slice.
2. **User flagged "UI still doesn't look like render A"** at the end of
   the previous session. That is exactly what items 11, 13, 19 are for —
   start there.
3. **Item 8 fade-in timing is instant pop-to-60%.** Spec was ambiguous;
   builder picked the literal reading. If the user wants a soft ramp,
   ~80ms ease-in matches OS conventions. Flag for design-lead in the
   next visual review.
4. **Item 7 nesting is stub-only.** Expanding a directory toggles the
   chevron and stores the index in `expanded_dirs` but does NOT render
   child rows. Real nested expansion + 8px-per-depth indent is a future
   task (the spec called it out as deferred).
5. **Branch still unpushed (7 commits ahead of `origin/rust-port`).**
   User has not authorized a push. Ask before pushing.

## Recommended next slice (items 11–14 + 19)

This is the gap-closer. Reorder vs. Hermes's original numbering so the
visual-diff pass happens early and informs everything else:

1. **Item 19 first — visual diff vs. render A.** Dispatch `design-lead`
   to load both the current screenshot and the target "render A" image,
   list the biggest deltas in priority order, and produce a delta spec.
   This becomes the authoritative input for items 11–14. Without it,
   items 11–14 risk drifting from what the user actually wants.
2. **Item 11 — editor chrome tighten.** File chip, header, spacing,
   active top accent. Specifics depend on item 19 output.
3. **Item 13 — editor typography hierarchy.** Content readable; chrome
   quieter. Likely overlaps with item 11; consider bundling.
4. **Item 12 — README. truncation bug.** Basename should preserve
   extension. Small, isolated; can be a parallel builder dispatch.
5. **Item 14 — real tab / open-buffer UI for the native editor.** Larger;
   may need its own design-lead spec before code.

Item **18** (right-side context panel strategy) is a **design + product
decision**, not code. Dispatch `design-lead` (or `product-strategist` if
the question is "do we even keep it?") before any code in that surface.

## Backlog (items 15–17, 20)

- **15.** Re-smoke Cmd+E idempotency screenshot.
- **16.** Multi-file open smoke (README → Cargo.toml, same surface reuses).
- **17.** Save/dirty indicator polish in tab/chrome/status line.
- **20.** Full closeout pass (fmt/clippy/test + screenshot suite).

These slot into the third slice after items 11–14 + 19 close the visual
gap.

## Routing recommendation for the next agent

1. **orchestrator** (per AGENTS.md orchestrator-first rule).
2. **Manual smoke first.** Run `scripts/run.sh`. Observe Explorer hover
   tracking, directory chevron toggle, scroll thumb fade, Outline header
   when empty. Capture a screenshot for item 19.
3. **design-lead** — visual diff against render A; produce delta spec
   for items 11, 13, 14 (and confirm items 5/7/8/10 actually look right
   in the running app).
4. **builder** — execute items 11, 13 first (likely bundled), then 12
   in parallel, then 14 against its own sub-spec.
5. **reviewer** — closeout before next handoff.

## Open questions for the user / next session

- **Push `rust-port` upstream?** Branch is 7 commits ahead; safe and
  small. Default recommendation: push now so the visible work isn't lost
  to a local mishap, and so any parallel review can see it.
- **Render A source-of-truth path.** Where does the "render A" target
  live? (Screenshot path? Figma? An earlier commit's rendered output?)
  design-lead needs to know before running the visual diff.
- **Item 18 — right-side context panel.** Keep, summon-on-demand, or
  remove? Needs a product call.
- **Item 7 nested expansion.** When is real child-row rendering wanted?
  Slot it before or after items 11–14?

## Useful pointers

- Items 7-10 spec: `context/2026-05-25-ide-polish-items-7-8-10-spec.md`
- Items 1-6 spec (inherited tokens/dimensions):
  `context/2026-05-25-ide-polish-slice-decisions.md`
- Original IDE redesign target: `context/2026-05-24-ide-redesign.md`
- Previous handoff (now superseded):
  `context/2026-05-25-ide-polish-handoff.md`
- Brand: `BRAND.md` (mineral palette, status semantics).
- Theme tokens: `crates/anvil-theme/src/theme.rs` (MINERAL_DARK,
  MINERAL_LIGHT).
- Files most likely to change next: `crates/anvil-render/src/` (editor
  surface, tab chrome), `crates/anvil/src/main.rs` (editor state, tab
  bookkeeping).
