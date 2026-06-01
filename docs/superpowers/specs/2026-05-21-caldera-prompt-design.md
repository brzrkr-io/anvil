# Caldera Prompt — Design Spec

> Status: draft for review. A scoped sub-project toward Anvil's terminal experience.
> Created: 2026-05-21. Owner-approved direction; pending spec review.

## Goal

Give Anvil its own shell prompt — **"the prompt that evolves with you."**
It replaces ad-hoc prompts (Starship and the like) with one that is:

- **Adaptive** — shows the segments the current directory and environment call
  for, nothing more.
- **Extensible** — absorbs new tools as the user adopts them, so the prompt is
  the constant while the toolset churns.
- **Sleek** — a coherent icon set, a calm two-line layout that collapses after
  use.
- **Discoverable** — hover any segment inside Caldera to reveal its detail in a
  clean, non-invasive popover; nothing is hidden, nothing is crowded.
- **Zig** — a small program shipped with Caldera and auto-wired by the existing
  shell integration.

## Context

M2 shipped shell integration: `src/app/shell_integration.zig` writes embedded
zsh/bash scripts to `~/.cache/anvil/shell/` and wires spawned shells
via `ZDOTDIR` (zsh) or an opt-in source (bash). Those scripts emit OSC 133 marks
and OSC 7 cwd, and **append `133;B` to the user's existing `PS1`** — so today
Caldera does not own the prompt. This sub-project makes Caldera provide it. The
integration also exports `ANVIL=1` into every spawned shell — reused
here to detect "running inside Caldera."

The config system (`src/config/config.zig`, ZON) and theme system
(`src/config/theme.zig`) are in place; this adds a `prompt` config section and
draws colors from the active theme.

## Decisions (settled with the owner)

1. **A standalone Zig program — `anvil-prompt`.** Built as a second executable
   by `build.zig`, source under `src/prompt/`. The shell integration installs it
   and wires the shell to call it each prompt draw. It emits the prompt as
   **ANSI text**: portable (SSH, tmux, any terminal), graceful degradation, no
   webview — the terminal-first, native-coherence bet. *Rejected:* rendering the
   prompt inside the Caldera app process — not portable, and the shell program
   covers the need.

2. **Two-line while live, transient after.** The active prompt is a two-line
   block — a context line (icons + segments) and a clean input line. On command
   submit the prompt **redraws collapsed** to a single bare `❯ <command>` line,
   so scrollback stays calm. zsh: a `precmd` hook for the full form plus a
   transient redraw on line-finish. bash: best-effort via `PROMPT_COMMAND`; if
   the clean transient redraw is not achievable on bash, the prompt simply does
   not collapse there (still correct).

3. **Low-profile accent edge.** A mineral vertical edge (a `▎`-class block in the
   theme accent color) opens both lines of the *active* block — the
   "low-profile agent input" marker. The collapsed transient form drops it.

4. **Adaptive segments.** `anvil-prompt` inspects the working directory and
   environment and shows only the segments that matter:
   - Always (in context): **repo / cwd**, **git** (branch, dirty count,
     ahead/behind).
   - Conditional: **toolchain / version** (when a `build.zig.zon`,
     `package.json`, `.tool-versions`, etc. is nearby), **container** (a
     `Dockerfile` / compose file), **cluster** (k8s manifests or an active
     kubeconfig context), **last result** (non-zero exit, or duration over a
     threshold), **time**.
   A clean directory shows almost nothing; a failed command reddens the edge and
   the prompt glyph.

5. **Extensible segments.** Beyond the built-ins, the user declares custom
   segments in `config.zon` — each `{ icon, command, when }`: a command whose
   output becomes the segment value, shown under a stated condition. This is the
   concrete "evolves with you" — adopting a new tool means adding a segment, not
   switching prompts. (Integration/plugin-pushed segments are deferred — see
   Non-goals.)

6. **A coherent icon set, via a bundled icon font.** Caldera bundles a small
   curated icon font (a private-use glyph range: repo, branch, cluster, cloud,
   container, checks, version, alert, time, …); the renderer's font stack
   includes it. `anvil-prompt` emits icon codepoints **when `$ANVIL`
   is set**, and a plain-text fallback (short labels / ASCII) otherwise — so the
   prompt is still clean in any other terminal. One drawn family, consistent
   weight and corners, so many integrations still read as one product.

7. **Config + theme.** A `prompt` section in `config.zon` controls which
   segments show, their order, the custom segments, and on/off. Colors resolve
   from the active theme. It live-reloads with the rest of config.

8. **An interactive layer — hover a segment for more.** Inside Caldera,
   hovering a prompt segment reveals its detail in a clean, non-invasive
   popover: pipeline → stage-by-stage status; git → ahead/behind, last commit,
   stash count; time → date, timezone, session uptime; and the same for any
   custom segment. **Mechanism:** when `$ANVIL` is set,
   `anvil-prompt` emits — after the visible prompt — a **metadata OSC** (a
   private Caldera OSC) carrying a small JSON payload: for each segment, its
   `id`, its column extent on the prompt row, and a `detail` payload. Caldera's
   VT parser captures it and tags it to the active prompt row (located via the
   OSC 133 `B` mark). On mouse hover over a segment's cells, Caldera draws a
   small popover anchored to that segment. The popover never steals focus,
   never reflows the terminal, and dismisses on mouse-out. In any other
   terminal the metadata OSC is silently ignored — the prompt stays a plain,
   static line. The popover is **native, raster-drawn** — consistent with how
   M2 already draws the tab bar and search bar (`render/tabbar.zig`,
   `render/searchbar.zig`).

## Architecture

A second build artifact and a new source tree, plus changes to shell
integration, config, and the bundled font.

| Path | Responsibility | Unit-tested |
|---|---|---|
| `src/prompt/main.zig` | `anvil-prompt` entry — parse args (full vs `--transient`, `--exit`, `--duration`), orchestrate, emit ANSI. | partial |
| `src/prompt/context.zig` | Detect directory/environment context — files present, git presence, kubeconfig, language markers. Pure logic. | yes |
| `src/prompt/segments.zig` | Segment model + built-in segments + custom-segment evaluation + active-set selection. Pure logic. | yes |
| `src/prompt/git.zig` | Fast git queries (branch, dirty, ahead/behind) via `git` plumbing with a hard timeout. | parsing only |
| `src/prompt/render.zig` | Compose segments → ANSI; full and transient forms; rich (icon) vs ASCII fallback. Pure logic. | yes |
| `src/prompt/icons.zig` | Icon codepoint table + ASCII fallbacks. Pure logic. | yes |
| `build.zig` | Add the `anvil-prompt` executable target; bundle the icon-font asset. | — |
| `src/shell/*.{zsh,bash}` | Install `anvil-prompt`, set `PS1`/`precmd`, wire the transient redraw. | — |
| `src/config/config.zig` | A `PromptCfg` config section. | yes |
| icon-font asset + `src/render/font.zig` | Bundle the icon font; add it to the renderer's font fallback stack. | — |

`anvil-prompt` is its own executable so it stays tiny and fast and has no
dependency on the running app process.

**The interactive layer** (decision 8) adds these, Caldera-side:

| Path | Responsibility | Unit-tested |
|---|---|---|
| `src/prompt/render.zig` (extends) | Also emit the segment-metadata OSC in rich mode. | yes |
| `src/terminal/parser.zig` + `terminal.zig` | Recognize the Caldera prompt-metadata OSC; expose the segment metadata for the active prompt row. | yes |
| `src/app/` input handling | Mouse-move tracking + hit-testing a hover against the active prompt's segment extents. | yes (hit-test) |
| `src/render/promptpopover.zig` | Raster-draw the segment-detail popover, anchored, dismissible — sibling to `tabbar.zig` / `searchbar.zig`. | — |

## Data flow

Each prompt draw: the shell hook calls
`anvil-prompt --exit $? --duration <ms> [--transient]`. The program →
`context` detects what is around → `segments` computes the active segment set
(built-in + custom) → `git` fills the git segment (timeout-guarded) → `render`
emits ANSI: rich icon glyphs when `$ANVIL` is set, ASCII fallback
otherwise; the full two-line block, or the collapsed line when `--transient`. On
command submit, the shell's transient hook re-invokes with `--transient` to
redraw the just-finished prompt collapsed.

**Interactive layer:** in rich mode, `render` also emits a segment-metadata OSC
after the visible prompt — each segment's `id`, column extent, and `detail`.
Caldera's parser captures it and stores it against the active prompt row. On a
mouse hover whose cell falls inside a segment's extent, Caldera raises the
popover with that segment's `detail`; mouse-out dismisses it.

## Performance

The program runs on every prompt — target well under ~30 ms. `git` queries use
plumbing commands with a hard timeout; on timeout the git segment is omitted (or
shown stale) rather than blocking. Context detection is a bounded set of `stat`s
on the current directory, not a tree walk. No segment may block the prompt.

## Error handling

- `anvil-prompt` missing or crashing → the shell scripts fall back to a
  minimal built-in `PS1`; the shell is never left broken.
- git slow, or not a repository → git segment omitted; no error surfaced.
- A custom segment's command fails or times out → that segment is skipped
  silently.
- Not inside Caldera (`$ANVIL` unset) → ASCII fallback glyphs; fully
  functional.
- bash cannot do a clean transient redraw → the prompt does not collapse on
  bash; still correct.

## Testing

- `context.zig`, `segments.zig`, `render.zig`, `icons.zig` — pure logic, fully
  unit-tested: context detection from a synthetic directory listing, segment
  selection per context, ANSI output (rich + fallback), full vs transient forms,
  the failure-state styling.
- `git.zig` — parsing tested against captured `git` output fixtures; the
  subprocess + timeout path verified manually.
- Interactive layer — unit-tested: the segment-metadata OSC encode/decode, the
  parser capturing it, and hover hit-testing (a cell → which segment, or none).
  The popover rendering and the hover interaction are verified manually.
- Manual: build and run; observe the prompt adapt across a git repo, a Node
  directory, and a k8s directory; confirm the failed-command styling; confirm
  the transient collapse; confirm the ASCII fallback in a non-Caldera terminal;
  hover each segment and confirm the popover is clean, anchored, and dismisses.

## Phasing

The spec covers the whole design; implementation splits in two:

- **Phase 1 — the prompt.** The `anvil-prompt` program: adaptive + extensible
  segments, the two-line transient layout, the icon set, config. Shippable and
  useful on its own.
- **Phase 2 — the interactive layer.** The segment-metadata OSC, the parser
  capture, hover hit-testing, and the popover. Adds Caldera-side terminal work;
  builds on Phase 1.

## Non-goals (deferred)

- Integration / plugin-pushed segments (needs the IPC and plugin work) — v1
  extensibility is config-declared commands only.
- Click-to-act on a segment (e.g. click the pipeline to open it) — hover-reveal
  only for now.
- The prompt learning usage and re-ordering itself — a later evolution.
- The workspace HUD / docks / panels — a separate, larger effort; this prompt
  stands alone and does not depend on it.
- Rendering the prompt inside the app process — the shell program covers it.

## Relationship to product direction

"The prompt that evolves with you" — adaptive + extensible — is a scoped,
shippable feature on its own, and also a concrete proof of a broader thesis: a
tool that absorbs a churning stack instead of being re-chosen every year. That
thesis is a candidate for the product's positioning and is owned by the planned
strategy work; this spec is buildable independent of it.
