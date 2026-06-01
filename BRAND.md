# Anvil Brand Contract

Status: active direction  
Updated: 2026-05-30  
Source: Anthropic Design handoff `caldera - 2`, `Caldera Brand Exploration.html`

## Purpose

Anvil is the app identity. Every app, tool, dashboard, CLI surface, document, report, landing page, and generated design artifact should feel like it belongs to Anvil even when the surface mode changes.

Use this file as the brand contract before any user-facing work. Product repos may copy this file and the files in `brand/`, but standalone repos must not depend on this private `caldera-os` workspace at runtime.

## Position

Anvil builds a developer platform and AgentOps control plane for agentic software work.

Tagline direction:

> The control layer for agentic software work.

The brand should feel:

- Modern developer infrastructure.
- Calm, precise, technical, and premium.
- Local-first, inspectable, and trustworthy.
- Powerful but controlled.
- Serious enough for platform engineering teams.

Dashboard and control-room surfaces should specifically evoke:

- Hermes-style agent/operator cockpit: alive, technical, command-adjacent, and built for supervision.
- Hacker/operator console density: dark tactical grid, monospace labels, thin borders, compact high-signal modules, trace IDs, terminal/code motifs, and visible system state.
- 0xide-like low-level technical confidence: sharp panels, sparse accents, inspectable runtime details, and no generic SaaS softness.
- Honcho.dev-like state/context workbench: persistent memory/context, agent state, handoffs, and next-action surfaces arranged as an operational map.

This is not decorative cyberpunk. It is an enterprise operator console: serious, local-first, auditable, and useful under pressure.


The brand should not feel:

- Hype SaaS.
- Crypto.
- Generic neon cyberpunk or decorative terminal cosplay.
- Playful consumer AI.
- Literal volcano software.

## Ecosystem Names

Use these names until a later naming decision replaces them:

| Role | Name |
| --- | --- |
| Company / parent brand | Anvil |
| Main platform / suite | Anvil Control |
| Desktop app | Anvil |
| CLI placeholder | Anvil CLI |
| Web dashboard placeholder | Control Room |
| Operational state layer | Anvil State |
| Event and provenance history | Trace |
| Starter/template pack | Anvil Kit |
| Adapters and integrations | Connectors |
| Rules and guardrails | Policy |
| Execution layer | Runtime |
| Provider/router layer | Gateway |

Low-level workflow nouns should stay plain: runs, lanes, checks, handoffs, attention, risks, evidence, actors, sessions, branches, and traces.

## Logo

The selected mark is **Basin**.

Geometry:

- 24-unit square grid.
- Outline circle centered at `(12, 12)`.
- Circle radius: `10`.
- Stroke width: `2`.
- Lower hemisphere filled solid.
- The flat top of the fill sits on the diameter at `y = 12`.
- No gradients, shadows, bevels, or 3D effects.

Reference SVG:

```svg
<svg viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
  <circle cx="12" cy="12" r="10" stroke="currentColor" stroke-width="2" fill="none"/>
  <path d="M 2 12 L 22 12 A 10 10 0 0 1 2 12 Z" fill="currentColor"/>
</svg>
```

Approved uses:

- Symbol-only app icon.
- GitHub/org avatar.
- Header lockup reading `Anvil`.
- Product lockups such as `Anvil Control`.
- Monochrome ink on bone/cream, or bone/cream on graphite.

Do not:

- Turn the Basin into a literal volcano, mountain, lava pool, flame, crater illustration, or mascot.
- Use orange fill as generic decoration.
- Add gradients, glow, bokeh, glass, 3D, bevels, badges, shields, or complex illustration.
- Replace the mark with anvils, hammers, skulls, weapons, or generic forge imagery.

## Typography

Primary type:

- `IBM Plex Sans`
- Use for wordmarks, UI text, headings, navigation, and product surfaces.
- Preferred weights: `400`, `500`, `600`.

Monospace type:

- `IBM Plex Mono`
- Use for labels, code, CLI examples, trace IDs, run IDs, status metadata, hex values, and compact operational rows.
- Preferred weights: `400`, `500`.

Fallback stack:

```css
--font-sans: "IBM Plex Sans", -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
--font-mono: "IBM Plex Mono", "SFMono-Regular", Consolas, "Liberation Mono", monospace;
```

Wordmark:

- Use `Anvil` in Title Case by default.
- Use IBM Plex Sans Medium (`500`).
- Keep spacing tight but legible.
- Do not distort the letters aggressively.

## Palette

Default palette: **Mineral**.

The palette evolves as **Mineral Warm** as of 2026-05-30 — backgrounds shift from cool blue-graphite toward warm near-black with a brown-rosewood undertone; primary accent moves teal→coral-rose; ember becomes a clean burnt orange; text whites are near-neutral cream to preserve contrast; the semantic-state vocabulary (verified/attention/risk/failure/agent/info/trace) is unchanged, hues warm-recalibrated but mutually distinct.

Mineral is the safest operational primary because it supports trace/provenance and avoids making the whole brand orange. Ember is held back for active run/execution state only.

### Core Materials

| Token | Hex | Use |
| --- | --- | --- |
| `anvil.graphite` | `#0e0b0a` | Primary dark canvas, app chrome, dark docs header |
| `anvil.charcoal` | `#1c1614` | Panels, sidebars, raised dark surfaces |
| `anvil.ash` | `#3e3028` | Secondary dark surfaces, separators, quiet fills |
| `anvil.alloy` | `#8a8076` | Muted text, metadata, disabled labels |
| `anvil.mist` | `#d8cfc8` | Light borders, separators, quiet backgrounds |
| `anvil.bone` | `#f0ebe4` | Primary light canvas |
| `anvil.white` | `#fdf6ee` | Raised light panels only |
| `anvil.ink` | `#140e0a` | Primary text on light surfaces |

### Operational Accent

| Token | Hex | Use |
| --- | --- | --- |
| `anvil.accent.mineral` | `#c2614a` | Primary accent, trace, provenance, active nav, focused state |
| `anvil.accent.ember` | `#d4601e` | Active execution/run state only, used sparingly |

### Semantic Status

| Token | Hex | Meaning |
| --- | --- | --- |
| `status.info` | `#c2614a` | Neutral operational info, trace-adjacent metadata |
| `status.verified` | `#5a8c45` | Evidence-backed success, passing checks, accepted state |
| `status.attention` | `#b88220` | Reviewable warning, stale context, pending action |
| `status.failure` | `#b53a2e` | Failed check, invalid state, rejected execution |
| `status.agent` | `#8c5fa0` | Agent, automation, model activity |
| `status.risk` | `#b85a30` | Scope, ownership, adapter, or strategy risk |
| `status.trace` | `#c2614a` | Source, trace, provenance, observed history |

Color rules:

- Color communicates state, not decoration.
- Green means verified by evidence. Never use it as a generic primary button color.
- Mineral coral belongs to trace, provenance, info, active nav, and focused operational state.
- Ember means active execution only. The accent family reads pink-and-orange; this is the warm brand register, not a license to use orange as decoration.
- Amber/gold means attention, not failure.
- Red means actual failure.
- Violet identifies agent/automation. Do not make it the dominant brand color.

## Surface Modes

Anvil has one brand grammar and multiple surface modes.

### Product Apps

Use compact, operational layouts. The first screen should show current work state: objective, actor, repo, branch, runs, lanes, checks, risks, trace, and next action.

Use:

- Dense but readable tables.
- Timelines.
- Sidebars.
- Status chips only when they encode real state.
- Low chrome.
- Stable dimensions.
- Provenance close to claims.

Avoid:

- Landing-page composition inside product tools.
- Decorative dashboard cards.
- Gradient backgrounds.
- Full-page brand art where operational state belongs.

### CLI

CLI output uses terse semantic mode.

Use:

- Short labels.
- Stable status words.
- Plain, parseable output.
- JSON as machine contract.

Avoid:

- Decorative boxes unless they materially improve scanning.
- Color-only state.
- Marketing copy.

### Docs And Reports

Docs use reader mode.

Use:

- Bone/light surfaces.
- Evidence blocks.
- Compact metadata.
- Trace/source links near claims.
- Calm typography.

Avoid:

- Terminal cosplay in long-form docs.
- Dense app layouts where reading is the job.

### Dashboards / Control Room

Control Room surfaces use the richest information density and must preserve dedicated pages for distinct operator jobs. A strong baseline is Overview, Board, Agents, Evidence, Trace, and Admin. Do not collapse these into one generic landing page when the product needs persistent operational surfaces.

Use:

- Dark graphite/black tactical canvas with subtle grid texture or scanline structure.
- Monospace-heavy labels, trace IDs, route names, run IDs, terminal output, and compact status metadata.
- Thin bordered technical panels with squared or barely-rounded corners; avoid soft SaaS cards.
- High-signal modules: current objective, next action, active run, agent fleet, board lanes, evidence, risks, handoff, trace, and admin/runtime state.
- Terminal/code motifs when they carry real state: prompts, command strips, logs, raw JSON inspectors, route maps, event traces.
- Mineral/teal for trace, active navigation, focused operational state, and system signal.
- Ember only for active execution/run heat; amber for attention; red for failure; violet for agent/model identity.
- Sparse glow only around live status dots, active routes, or selected state. It should feel like Hermes / 0xide / Honcho, not a neon poster.

Avoid:

- Flattening the dashboard into a generic SaaS overview.
- Removing dedicated pages/tabs that separate Overview, Board, Agents, Evidence, Trace, and Admin jobs.
- Multi-accent decoration unrelated to state.
- “Lava dashboard” treatment.
- Generic dark SaaS cards without operational meaning.
- Marketing hero sections that displace current work state.

Use:

- Dark graphite canvas.
- Runs, lanes, state, Trace, handoffs, checks, attention, risks, and gateway status.
- Mineral or Ember only where state requires it.

Avoid:

- Multi-accent decoration.
- "Lava dashboard" treatment.
- Generic dark SaaS cards without operational meaning.

## Shape And Layout

Use compact system geometry:

| Token | Values |
| --- | --- |
| Grid | 24-unit mark grid |
| Radius | `4px`, `6px`, `8px` |
| App icon radius | about `22%` of icon size |
| Control heights | `28px`, `32px`, `38px`, `44px` |
| Spacing | `4px`, `8px`, `12px`, `16px`, `24px`, `32px`, `48px`, `64px` |

Prefer thin borders, strong alignment, and compact rows before shadows.

## Agent Rules

Before app, CLI, dashboard, design, theme, UI, docs/report layout, landing-page, slide, or user-facing surface work:

1. Read this `BRAND.md`.
2. Read `docs/design/shared-core-surface-modes.md` when working inside `caldera-os`.
3. Use the Basin mark, IBM Plex type system, Mineral palette, and surface-mode rules unless the user explicitly approves a different direction.
4. If creating a standalone repo, copy the relevant brand assets/tokens into that repo. Do not import them from `caldera-os` at runtime.
5. Treat screenshots or AI design exports as evidence. Convert durable brand decisions into files: `BRAND.md`, SVG assets, CSS tokens, JSON tokens, and local design docs.

When uncertain, choose the calmer, more operational option.

## Assets

Canonical local assets live in `brand/`:

- `brand/mark.svg`
- `brand/mark-inverted.svg`
- `brand/lockup.svg`
- `brand/app-icon.svg`
- `brand/avatar.svg`
- `brand/favicon.svg`
- `brand/tokens.css`
- `brand/tokens.json`
