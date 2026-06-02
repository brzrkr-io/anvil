# Anvil Roadmap

Stack: **Tauri v2 (Rust) + SvelteKit**. No GPUI, no native rewrite — stay Tauri.
Caldera integration is **out of scope** (removed).

Reality check (2026-06): the codebase is already mature — terminal core, PTY
hardening, editor+LSP, git/SCM, k8s/CI/IaC/observability, AI agent v2, themes,
workspace, accounts are all shipped. So this roadmap is mostly **harden + close
real gaps + lock quality**, not greenfield. Audit before building each item;
don't rebuild what exists.

## Done this pass
- [x] Clean `svelte-check` to 0 errors / 0 warnings (a11y, deprecations, unused CSS)
- [x] CI fails on warnings (svelte-check `--threshold warning`, clippy `-D warnings`)

## P0 — stability & quality (the app reads as flaky; fix that first)
1. Translucency: make frost work in Tauri **or** default to a clean solid. Current
   native vibrancy (`hudWindow`) + semi-transparent surfaces does not read as true
   frosted glass. Decide and finish — don't leave it half-working.
2. Rust panic → surfaced + recovered in UI, never white-screen.
3. Session/layout/cwd/terminal restore — reliable across relaunch AND crash.
4. PTY robustness pass: child-death respawn, EAGAIN backpressure, output-flood (mostly done — verify under load).
5. Long-session memory audit (xterm buffers, listeners, SWR eviction).
6. Multi-window state isolation — no cross-window bleed.
7. Close/focus edge cases (last tab, empty states, split focus).
8. Local opt-in crash/error telemetry to actually find bugs.
9. Raise test coverage toward 90% (unit + Playwright e2e on key flows).
10. Add Playwright e2e to CI (headless display).

## P1 — AI agent depth (the differentiator)
11. Tool-approval cards: read auto, write/exec confirm (verify the existing flow is solid).
12. Sub-agents — named, own prompt + tool subset.
13. Agent project memory (read AGENTS.md / repo map as context).
14. Inline editor AI autocomplete + accept/reject diff hunks.
15. Agent acts on terminal/k8s/CI failures ("explain + fix this").
16. Background agent tasks + completion notification.
17. Redaction/secret-scrub audit on everything sent to the LLM.
18. Local model path (LM Studio/Ollama) verified end-to-end.

## P1 — DevOps depth (the user's actual job)
19. k8s: logs stream / exec / port-forward / describe / events polish + apply-diff-confirm.
20. k8s: context+namespace switcher in status bar.
21. CI: pipeline view + re-run + live logs (GitHub Actions + GitLab).
22. Terraform/Terragrunt: plan/apply with diff + confirm.
23. Helm: list / diff / rollback.
24. Observability (SigNoz/Prometheus/Grafana): harden the wired panels.
25. Incident / on-call view.
26. Cloud (AWS) resource browser via Keychain creds.
27. Docker/OrbStack containers + compose.

## P1 — git/SCM depth
28. Hunk-level stage/unstage/discard.
29. Interactive rebase / amend / fixup UI.
30. Conflict-resolution editor.
31. PR create/review inline (GitHub + GitLab).
32. Blame gutter + line history.

## P2 — UX / navigation
33. Finish + merge unified tab model (`terax-unified-tabs`).
34. Command palette: every action, fuzzy, recent — deepen.
35. Go-to-anything: files + symbols + commits + commands.
36. Named workspace layouts save/restore.
37. Keybinding editor + presets (vscode/zed/vim).
38. Clickable context chips in status bar (git/k8s/cloud).

## P2 — platform / release
39. `anvil` CLI verb + `anvil://` deep links hardening.
40. Per-monitor / fullscreen / Stage Manager correctness.
41. Auto-update + signed/notarized release pipeline.
42. macOS notifications + menu-bar presence.

## P2 — security
43. Secret deny-list on read + write paths (audit).
44. Keychain-only credentials everywhere (no localStorage secrets).
45. CSP + IPC boundary validation review.
46. Prompt-injection defense for agent tool use.

## P3 — product / distribution
47. Onboarding that teaches the `+` model + agent.
48. In-app changelog + tips.
49. Marketing site parity (anvil.brzrkr.io).
50. Define the wedge user + the 3 workflows Anvil must beat terminal+VSCode+kubectl at.

Note: this is a deliberately shorter, de-duplicated list than the raw "100" —
many of the original 100 were already shipped. Work the genuine gaps above.
