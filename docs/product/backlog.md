# Anvil — Backlog (Zig)

Ranked next-work list. Quality bar for the terminal core: Ghostty / Alacritty.
Tiers run correctness → usability → polish → surfaces. Build tier 1 first.

Audited state (2026-05-29):

- **Done:** truecolor + 256 + indexed SGR, bold/underline/reverse attrs (cell),
  scrollback + wheel scroll, selection + copy, tabs, splits, command palette,
  theme live-reload, dynamic lazy glyph atlas (Unicode font-cascade).
- **Missing:** see items below.

Dependency spine: **#1 (OSC) unlocks #9, #12, #17.** Highest-ROI single item.
Recommended order: 1 → 3 → 2 → 4, then the rest.

---

## Tier 1 — terminal correctness (blocks "daily driver")

1. **OSC handler.** Parser has no `]` state. Add OSC dispatch: window title
   (0/2), cwd (7), hyperlinks (8), clipboard (52), shell-integration marks (133).
   Foundational — several items depend on it.
2. **Mouse reporting.** SGR 1006 + 1000/1002/1003. Without it vim/tmux/htop
   clicks are dead.
3. **Wide-char width.** wcwidth: CJK + emoji = 2 cells. Today every codepoint
   is 1 cell → misaligned output. Atlas already does Unicode; the grid does not
   do width.
4. **Bracketed paste** (2004) + Cmd+V paste path. Safe multiline paste.
5. **Alt-screen + scrollback save/restore** (1049). vim/less must not corrupt
   scrollback. Verify complete.
6. **Italic / dim / strike / blink attrs.** Cell has only bold/underline/reverse.
   Add the missing SGR attrs end to end.

## Tier 2 — daily usability

7. **Render bold + underline + strike.** Confirm the renderer draws them (bold =
   weight/bright, underline/strike = rule). Parsed ≠ shown.
8. **Search in scrollback** (Cmd+F). No search in `src/` yet.
9. **Tab labels = cwd / title** (needs #1). Replaces bare numbers.
10. **Cursor styles.** bar/underline/block + blink, DECSCUSR.
11. **Config expansion.** Font family/size, padding, keybinds, cursor. Live-reload
    covers theme only.
12. **Clipboard via OSC 52** (needs #1). Remote/tmux copy.

## Tier 3 — visual polish

13. **Font shaping / ligatures** (HarfBuzz). Programming-font ligatures, combining
    marks.
14. **Atlas LRU eviction.** Fills 1024 slots then new glyphs fall back to blank.
15. **Cursor blink + focus dimming.** Unfocused pane cursor hollow.
16. **sRGB-correct text blend.** Verify gamma; thin text on dark.

## Tier 4 — surfaces (roadmap M3+)

17. **Shell-integration jumps** (OSC 133 marks, needs #1). Jump prev/next prompt.
18. **Splits polish.** Keyboard resize, zoom pane, balance.
19. **Webview host + typed IPC** (M3) — or stay native-Metal per the "native only"
    call. Decision gate for browser/editor/agents.
20. **Agent surface, reframed.** Ambient terminal-native (inline run blocks), not
    a docked dashboard.

## Tier 5 — user-reported polish (do at the end)

21. **Smooth window resize/drag.** Live-resize is glitchy; redo for smooth reflow
    of grid + PTY with better visuals.
22. **Nerd-font icons.** Bundled `BlexMonoNerdFontMono-Regular.ttf` is unused — the
    shim hardcodes Menlo. Switch the atlas to the Nerd Font so PUA icon glyphs
    render (lazyvim/devicons).
23. **Solid vim/nvim/lazyvim support.** End-to-end: truecolor, mouse, alt-screen,
    icons, cursor shapes, undercurl, fast resize redraw.
