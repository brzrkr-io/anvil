---
date: 2026-05-24
kind: design-spec
status: approved
goal: HUD redesign — DevOps + AI-native section set, graphite palette, pixel-positioned layout
touches: crates/anvil-render/src/agent_panel.rs::draw_right_hud
---

# HUD Redesign Spec

## Purpose

Replace the current REPO/GIT/AGENTS/SYSTEM section set with one tuned for a DevOps
engineer using an AI-native console. The HUD should read operational state at a
glance: where you are (cluster, repo), what is broken (CI, dirty git), what agents
are doing, and what the system is doing — in that priority order.

## Section List and Order

Ordered top-to-bottom by attention priority for a DevOps workflow.

### 1. CONTEXT
K8s context/namespace. Highest urgency — wrong cluster = wrong blast radius.

Rows:
- env-tint dot · cluster name (foreground) · `·` · namespace (meta tone)
- Env tint: cluster name contains "prod"/"prd" → ATTENTION amber dot; "stg"/"staging" → INFO_TEAL dot; else ALLOY dot.
- Click: copy full context string. Hidden when no kubectl or no context.

Data source: new `LocalContext.kube_context: Option<(String, String)>` populated by background `kubectl-context` poller.

### 2. REPO + GIT (merged, was two sections)
Saves one section header row vs the old two-section split.

Rows:
- Row 1: repo basename (foreground)
- Row 2: parent path (meta tone)
- Row 3: `⎇` (INFO_TEAL) · branch name (foreground) · ahead/behind badge OR `*N modified` (ATTENTION)
- Row 4: HEAD short SHA (INFO_TEAL) · commit subject (meta tone). Click SHA → copy.

Collapsed: shows the header row + the branch row only.

### 3. CI
Last CI run on current branch + open PR count. Replaces old LAST RUN / BUILD.

Rows:
- Row 1: status glyph + label: `✓ main · 2m14s` (VERIFIED), `✗ main · 1m02s` (FAILURE), `● running…` (accent-bright teal), `· no data` (ALLOY).
- Row 2 (when open_prs > 0): `2 open PRs` (INFO_TEAL). Cmd-click opens PR URL.

Data source: new `LocalContext.ci_status: Option<CiStatus>` populated by `gh-ci` poller. Hidden when None.

### 4. AGENTS
Unchanged semantics. Caldera connection dot + summary line + priority rows (approvals → running runs → failure findings). Idle state: dot VERIFIED green, label "idle".

### 5. RECENT
Existing `recent_files` list, max 5 rows. Collapsed hides all rows. No logic change.

### 6. PORTS
Existing `ports` list. Shown only when non-empty.

### 7. SYSTEM (demoted, compact)
Single row only: `mem ▄▅▆▆▃▁ 9/16 GB · load 1.42`. Drop disk row. Drop clock (belongs in bottom bar).

## SectionId Changes

- Add: `SectionId::Context`, `SectionId::Ci`, `SectionId::RepoGit`
- Remove: `SectionId::Repo`, `SectionId::Git`, `SectionId::LastRun`, `SectionId::Build`
- New `DEFAULT_ORDER`: `[Context, RepoGit, Ci, Agents, Recent, Ports, System]`

## Visual Treatment

**Section header**: existing `draw_section_header` (`─ LABEL ────────` form). Label color `tones.label`. No change.

**Inter-section rule**: existing `section_break` 1px hairline at `tones.edge`. No change.

**Status glyphs**: U+2713 `✓` VERIFIED; U+2717 `✗` FAILURE; U+25CF `●` accent-bright for running; U+00B7 `·` ALLOY for no-data. U+25CF bullet for env-tint dot.

**Collapse state**: `u8` bitmask on `App`, one bit per `SectionId`. Persisted under `[hud.collapsed]` in TOML config. Collapsed section renders header row only.

**Dirty-row gating**: CI and kubectl data update on 10s / 30s TTL via background pollers. Each poller writes to an `Arc<Mutex<Option<T>>>`. `draw_right_hud` reads with `try_lock`; on contention, the previous frame's cached value is used. No `force_full_redraw` triggered by HUD ticks.

## Palette Token Map

All existing constants from `agent_panel.rs`. No new colors.

| Element | Token | Hex |
|---|---|---|
| Surface | GRAPHITE @ 0.88 alpha | `#0b0d0e` |
| Left hairline | `tones.edge` | `#2a303c` |
| Section headers | `tones.label` | `#6b7682` |
| Body text | `tones.foreground` | `#d6dce4` |
| Metadata / parent / timestamp | `tones.meta` / ALLOY | `#86919a` |
| Branch glyph, SHA, ports, open PRs | INFO_TEAL | `#3a8a9d` |
| Clean git state, CI pass | VERIFIED | `#3f8a5b` |
| Dirty files, prod env dot, pending approvals | ATTENTION | `#b07a14` |
| CI failure, findings | FAILURE | `#b13a30` |
| Agent/automation activity | AGENT_VIOLET | `#6a5fa3` |
| Running CI / active cursor | accent-bright | `#54b7c0` |

## Toggle

Cmd+`\` toggles HUD visibility. No change. Collapse state is independent of visibility.

## New Data Structs

```rust
pub struct CiStatus {
    pub state: CiState,
    pub branch: String,
    pub duration_s: u32,
    pub open_prs: u32,
    pub pr_url: String,
}
pub enum CiState { Running, Ok, Failed, Unknown }
```

`LocalContext` gains:
- `kube_context: Option<(String, String)>` — (cluster, namespace)
- `ci_status: Option<CiStatus>`

## Builder-executable note

Touches `agent_panel.rs::draw_right_hud` (new CONTEXT and CI sections, REPO+GIT merge, SYSTEM to single row, new `SectionId` variants, new `LocalContext` fields) and the `LocalContext` struct definition.

New workers required:
1. **`kubectl-context` poller** — `kubectl config current-context` + namespace, 30s TTL, `Arc<Mutex<Option<(String,String)>>>` write. Checks `which kubectl`; fails silent when absent.
2. **`gh-ci` poller** — `gh run list --branch <branch> --limit 1 --json status,conclusion,createdAt,durationMs` + `gh pr list --head <branch> --json url --jq 'length'`, 10s TTL, same pattern. Checks `which gh`; fails silent when absent.

Worker crate placement: defer to systems-architect pass (will be covered by the in-flight `2026-05-24-polish-architecture.md` which already plans the kubectl worker under item 20).
