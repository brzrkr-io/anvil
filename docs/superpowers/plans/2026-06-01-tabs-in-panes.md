# Tabs-in-Panes (§A #2) — Implementation Plan

> **For agentic workers:** execute task-by-task; each task ends green (svelte-check 0 errors + `vitest run` passing). Verify loop per `context/handoff-2026-06-01-roadmap-sweep.md`.

**Goal:** Each workspace pane (Leaf) holds multiple tabs (its own mini tab strip), like Zed editor groups — instead of one view per pane.

**Architecture:** Change `Leaf` from a single `{view, ref}` to `{ tabs: PaneTab[], active: number }` where `PaneTab = { view, ref?, id }`. Keep panes mounted (the verified `display:none` mount-stable technique from §A #8). PaneGrid renders a tab strip in each `.phead`. All `panes.ts` ops migrate to operate on the active tab.

**Tech stack:** Svelte 5 runes, `src/lib/panes.ts` (pure, unit-tested), `src/lib/PaneGrid.svelte`, `src/routes/+page.svelte`.

---

### Task 1: Extend the Leaf model (panes.ts)

**Files:** Modify `src/lib/panes.ts`, `src/lib/panes.test.ts`

- [ ] **Step 1 — failing test:** add to panes.test.ts
```ts
it("leaf holds a tab list with an active index", () => {
  const lf = leaf("term", "t1", "A");
  expect(lf.tabs.length).toBe(1);
  expect(lf.tabs[lf.active].view).toBe("term");
});
```
- [ ] **Step 2:** run `vitest run` → fails (no `.tabs`).
- [ ] **Step 3 — implement:** change `Leaf` to
```ts
export interface PaneTab { id: string; view: ViewKind; ref?: string }
export interface Leaf { kind: "leaf"; id: string; tabs: PaneTab[]; active: number }
```
Update `leaf(view, ref?, id?)` to `{ kind:"leaf", id: id ?? paneId("l"), tabs:[{id:paneId("tab"), view, ref}], active:0 }`.
Add helpers: `activeTab(lf) => lf.tabs[lf.active]`, `addTab(lf, view, ref)`, `closeTab(lf, idx)`, `setActiveTab(lf, idx)`.
- [ ] **Step 4:** keep `setView` operating on the **active tab** (replace active tab's view/ref). `splitLeaf`/`dockLeaf`/`closeLeaf` use `activeTab(lf).view/ref` where they read `.view`/`.ref`. `remapTermRefs` maps every tab's term ref.
- [ ] **Step 5:** run `vitest run` → all pass (update the existing 11 tests that read `.view`/`.ref` to read `activeTab(lf)`).
- [ ] **Step 6:** commit.

### Task 2: Render per-pane tab strip (PaneGrid.svelte)

**Files:** Modify `src/lib/PaneGrid.svelte`

- [ ] **Step 1:** in the leaf branch, replace the single `vpick` select with a tab strip: `{#each lf.tabs as t, i}` → a tab button (label from `labelOf(t.view)` or `baseName(t.ref)`), click sets active (`onSetActiveTab(lf.id, i)`), `×` closes (`onCloseTab(lf.id, i)`). Keep the existing split/close pane buttons.
- [ ] **Step 2:** `{@render view(lf)}` already gets the Leaf — update the `paneView` snippet in +page to render `activeTab(lf)` instead of `lf.view`/`lf.ref`.
- [ ] **Step 3:** add props `onSetActiveTab`, `onCloseTab`, `onAddTab` (thread through `<svelte:self>` recursion — mirror how `zoomId`/`extDrag` were threaded).
- [ ] **Step 4:** `svelte-check` → 0 errors; visually confirm a pane shows its tabs.
- [ ] **Step 5:** commit.

### Task 3: Wire handlers + dock-into-pane-as-tab (+page.svelte)

**Files:** Modify `src/routes/+page.svelte`

- [ ] **Step 1:** add `wsSetActiveTab`, `wsCloseTab` (call `setActiveTab`/`closeTab` on the leaf in paneTree via a tree map helper).
- [ ] **Step 2:** change `wsDropTab` (external tab drop) and `dockLeaf` **center** edge to *add a tab* to the target leaf instead of replacing its view — that's the real tabs-in-panes payoff (drop a top tab onto a pane center → becomes a new tab in that pane).
- [ ] **Step 3:** update `paneView` snippet to use `activeTab(lf)`.
- [ ] **Step 4:** `svelte-check` 0 errors + `vitest run` green + `vite build`.
- [ ] **Step 5:** browser-verify (Claude_Preview, restart server for fresh build): split a pane, drop a top tab on its center, confirm the pane now has 2 tabs and both underlying components stayed mounted.
- [ ] **Step 6:** commit; mark roadmap §A #2 `[x]`.

---

## Risks / notes
- **Persistence:** `paneTree` is in `write_state`; the new Leaf shape is a superset — add a migration in `remapTermRefs` so old saved trees (single-view leaves) upgrade to `{tabs:[…], active:0}` instead of crashing on load.
- **Mount stability:** the `paneView` snippet must render only the active tab's component; inactive tabs in the same pane are NOT mounted (acceptable — switching tabs remounts within a pane, like a browser). If terminal-session preservation across tab switches is required, apply the §A #8 `display:none` technique inside the leaf too (render all tabs, hide inactive).
- **Tests:** the 11 existing panes tests must be migrated in Task 1, not deferred.
