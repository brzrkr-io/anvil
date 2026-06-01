---
status: active
type: decision
created: 2026-05-29
updated: 2026-05-31
sources: []
confidence: high
---

# 0005 — Render Host: Native Metal, No Webview Chrome

## Status

Active. Decided 2026-05-29. Resolves backlog #19 (the webview-vs-native gate).

## Context

The console-rebuild backlog (#19) left open whether Anvil's host surface should
be a WKWebView with a typed native↔web IPC bridge (the archived Rust tree's
`anvil-control` model, with `ui/palette/` HTML chrome) or stay fully
native-Metal. The Zig rewrite (M0–M3 + tiers 1–4) has already built every chrome
surface — tab bar, dividers, command palette, search bar, cursor, overlays — as
native Metal instance/solid-rect draws. There is no webview anywhere in the Zig
tree.

Forces:

- **Performance / latency.** The whole point of the rewrite is a blazing-fast
  terminal at the Ghostty/Alacritty bar. A webview host adds a JS/DOM hop and a
  serialization boundary on the hot path for chrome that is already trivially
  expressible as quads.
- **Single render pipeline.** Chrome and terminal cells share one Metal frame
  (`anvil_frame`), one atlas, one draw loop. A webview would fork that into two
  compositors that must be kept in sync.
- **Standing direction.** Memory records `native-metal-only` and `ai-native`:
  chrome stays in Metal; AI/agents are an architectural concern, not a reason to
  embed a browser for UI.

## Decision

**The host is native AppKit + Metal. No WKWebView is used for app chrome.**

- All chrome (tabs, status, palette, search, dividers, agent surface) renders
  through the existing native pipelines.
- A webview is permitted **only** as the *content* of a dedicated pane type if a
  genuine web surface is needed later (e.g. a docs/browser pane or a rich agent
  output view). It is never the host and never renders app chrome.
- The typed native↔web IPC bridge (`anvil-control`-style Inbound/Outbound) is
  **deferred** until such a content pane actually exists. Agent input/output
  (#20) is delivered terminal-native (inline run blocks), which needs no web
  bridge.

## Consequences

- No spike work is required; the gate is closed in favor of the path already
  taken. #19 ships as this decision record.
- #20 (agent surface) proceeds as an ambient, terminal-native surface, not a
  docked webview dashboard.
- If a web *content* pane is later justified, it enters as an isolated pane
  content type plus a minimal typed bridge — a new decision at that point, not a
  reopening of the host question.

## Amendment — 2026-05-31

The escape hatch recorded above ("a webview is permitted only as the content of
a dedicated pane type") has been activated. Sub-project #1 of Anvil's hybrid
architecture landed on branch `zig` (commits d7e73bf..44f3c07).

**What shipped:**

- A live, navigable `WKWebView` now renders as the content of a `.web` pane type.
  It is an AppKit `NSView` subview layered into the NSWindow, positioned and
  clipped per-pane by `anvil_web_*` C exports in `src/platform/shim.m`. It is
  NOT composited into the Metal frame; Metal chrome and the WKWebView are sibling
  layers.
- App chrome stays 100% native Metal. A native Metal "header strip" (URL bar,
  back/forward/reload buttons, loading progress line) is drawn ABOVE the web body;
  no Metal chrome floats over web content.
- The typed native↔web bridge deferred by this decision is now realized in thin
  v1 form: native→web = navigate/back/forward/reload (C-ABI calls); web→native =
  KVO (title, estimatedProgress, canGoBack/canGoForward) + content-process-terminated
  crash delegate. No custom `WKScriptMessageHandler` actions (deferred to a later
  sub-project).
- Security guardrails: ephemeral `WKWebsiteDataStore` (no cookies/creds shared with
  native, no cross-session persistence); URL bar accepts http/https only.
- Invocation: command-palette action "Open Browser Pane" (`anvil_open_web`).

**What does NOT change:** the core decision stands. The host is native AppKit +
Metal. WKWebView is never the host and never renders app chrome. This amendment
records only that the previously-deferred content-pane escape hatch is now active.

Design spec: `docs/superpowers/specs/2026-05-31-webkit-content-pane-design.md`.
Plan: `docs/superpowers/plans/2026-05-31-webkit-content-pane.md`.
