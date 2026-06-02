---
status: active
type: concept
created: 2026-06-02
updated: 2026-06-02
sources: []
confidence: high
---

# Security Boundary (CSP + IPC)

Audit of the webview↔Rust trust boundary. The webview is untrusted-content
host; Rust commands are the privileged surface. Posture as of 2026-06-02.

## CSP

`tauri.conf.json > app.security.csp` is locked down:

- `default-src 'self'`, `object-src 'none'`, `frame-ancestors 'none'`,
  `base-uri 'self'` — no remote document/embed origins.
- `connect-src 'self' ipc: http://ipc.localhost` — network is local IPC only.
  No outbound origins; LLM/cloud calls go through Rust commands, not the webview.
- `script-src 'self' 'unsafe-inline'` and `style-src 'self' 'unsafe-inline'`.

`'unsafe-inline'` is **required, not laziness**:
- SvelteKit's adapter-static emits one inline hydration bootstrap `<script>` in
  `build/index.html`. Removing `'unsafe-inline'` (or hashing it) breaks hydration
  → white screen.
- xterm.js and CodeMirror inject inline `style` for the grid/gutter.

Accepted because every origin is local — there is no remote-content vector to
smuggle injected script out to. Revisit if a remote origin ever enters
`connect-src`/`frame-src`.

## Capabilities

`capabilities/default.json` grants only: `core:default`,
`core:window:allow-start-dragging`, `opener:default`, `updater:default`,
`window-state:default`. Scoped to `main` + `w*` windows.

- **No shell plugin.** There is no `shell:allow-execute`. The only shell exec is
  our own `run_capture` command (`sh -c <cmd>` in a cwd), which is the agent
  tool-use channel and is **UI-approval-gated** — see prompt-injection notes.
- **No `withGlobalTauri`**, no `dangerousRemoteDomainIpcAccess`. Default-deny.

## Command surface

~150 `#[tauri::command]`s across git/kube/iac/ci/flux/cloud/fs/pty/lsp/llm.
Structured commands build `Command` with explicit `.arg()` (no shell string
interpolation). Subcommand verbs that reach a shell are allow-listed
(`git op`, `flux_kind`, `ci` action, iac kind) so a crafted argument can't pivot
to an arbitrary verb.

`fs` commands (`read_file`/`write_file`/`list_dir`/…) take arbitrary paths by
design — Anvil is a local editor/IDE. Not a sandbox; trust model is "the user's
own machine, the user's own files."

## Findings

Boundary is sound for a local-first desktop app. The two residual items are
**by design**: `'unsafe-inline'` (local-only, hydration-required) and arbitrary
`fs` paths (editor requirement). The live attacker surface is the agent's
`run_capture` — covered by prompt-injection defense, not this boundary.
