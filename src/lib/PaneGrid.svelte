<script lang="ts">
  import { leafIds, type PaneNode, type SplitNode, type Leaf, type ViewKind, type Edge } from "./panes";
  import PaneGrid from "./PaneGrid.svelte";
  import type { Snippet } from "svelte";

  let {
    node,
    view,
    activeId = "",
    onSplit,
    onClose,
    onSetView,
    onResize,
    onFocusLeaf,
    onTabPointerDown,
    dropHint = null,
    onSetActiveTab,
    onCloseTab,
    onAddTab,
    zoomId = null,
    dim = false,
    depth = 0,
    solo = false,
  }: {
    node: PaneNode;
    view: Snippet<[Leaf]>;
    activeId?: string;
    onSplit: (leafId: string, edge: Edge, v: ViewKind, srcRef?: string) => void;
    onClose: (leafId: string) => void;
    onSetView: (leafId: string, v: ViewKind) => void;
    onResize: (splitId: string, index: number, deltaFrac: number) => void;
    onFocusLeaf?: (leafId: string) => void;
    // Pointer-drag a pane's own tab into another pane (drag-to-split/move, #4/#5).
    onTabPointerDown?: (e: PointerEvent, leafId: string, index: number) => void;
    // The pane+edge currently highlighted by an in-flight tab drag (owned by the
    // shell's pointer-drag controller). Renders the .dropzone overlay.
    dropHint?: { leafId: string; edge: Edge } | null;
    // Per-pane tabs (#2).
    onSetActiveTab?: (leafId: string, idx: number) => void;
    onCloseTab?: (leafId: string, idx: number) => void;
    onAddTab?: (leafId: string) => void;
    // When set, the subtree NOT containing this leaf is hidden (display:none),
    // so the zoomed pane fills the workspace — without unmounting any pane (#8).
    zoomId?: string | null;
    // Focus dimming (#65): fade inactive leaves.
    dim?: boolean;
    depth?: number;
    // True when the whole tree is a single leaf — hide the pane header so a lone
    // pane is a clean full-bleed view (no "floating workspace box" chrome).
    solo?: boolean;
  } = $props();

  const VIEWS: { k: ViewKind; label: string }[] = [
    { k: "term", label: "Terminal" }, { k: "editor", label: "Editor" },
    { k: "files", label: "Explorer" }, { k: "scm", label: "Source Control" },
    { k: "search", label: "Search" }, { k: "agent", label: "Agent" },
    { k: "devops", label: "DevOps" }, { k: "k8s", label: "Kubernetes" },
    { k: "ci", label: "CI / Pipelines" }, { k: "terraform", label: "Terraform" },
    { k: "obs", label: "Observability" },
  ];
  const labelOf = (k: ViewKind) => VIEWS.find((v) => v.k === k)?.label ?? k;

  // ── splitter drag (resize) ──
  let splitEl = $state<HTMLDivElement | undefined>(undefined);
  function startResize(e: PointerEvent, sp: SplitNode, index: number) {
    e.preventDefault();
    const rect = splitEl!.getBoundingClientRect();
    const total = sp.dir === "row" ? rect.width : rect.height;
    // Coalesce pointer moves to one onResize per animation frame (#20) so a fast
    // drag doesn't thrash layout/reflow on every mousemove event.
    let pending = 0;
    let raf = 0;
    const flush = () => { raf = 0; if (pending) { onResize(sp.id, index, pending / total); pending = 0; } };
    const move = (ev: PointerEvent) => {
      pending += sp.dir === "row" ? ev.movementX : ev.movementY;
      if (!raf) raf = requestAnimationFrame(flush);
    };
    const up = () => {
      if (raf) cancelAnimationFrame(raf);
      flush();
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }
</script>

{#if node.kind === "split"}
  {@const sp = node}
  <div class="split dir-{sp.dir}" bind:this={splitEl}>
    {#each sp.children as child, i (child.id)}
      {@const hide = zoomId && !leafIds(child).includes(zoomId)}
      <div class="cell" style="flex: {hide ? 0 : zoomId ? 1 : sp.sizes[i]} 1 0; min-width:0; min-height:0; {hide ? 'display:none;' : ''}">
        <PaneGrid node={child} {view} {activeId} {onSplit} {onClose} {onSetView} {onResize} {onFocusLeaf} {onTabPointerDown} {dropHint} {onSetActiveTab} {onCloseTab} {onAddTab} {zoomId} {dim} depth={depth + 1} />
      </div>
      {#if i < sp.children.length - 1 && !zoomId}
        <div class="divider dir-{sp.dir}" onpointerdown={(e) => startResize(e, sp, i)} role="separator" tabindex="-1"></div>
      {/if}
    {/each}
  </div>
{:else}
  {@const lf = node}
  <div class="leaf {lf.id === activeId && !solo ? 'active' : ''}" class:dimmed={dim && lf.id !== activeId} data-leaf-id={lf.id} onpointerdowncapture={() => onFocusLeaf?.(lf.id)}>
    {#if !solo}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="phead">
      <span class="grip">⠿</span>
      <div class="ptabs">
        {#each lf.tabs as t, i (t.id)}
          <!-- Pointer-drag (not HTML5) so a tab can be dragged into another pane
               to split/move; a sub-threshold press still selects the tab. -->
          <button class="ptab {i === lf.active ? 'on' : ''}" onpointerdown={(e) => onTabPointerDown?.(e, lf.id, i)} onclick={() => onSetActiveTab?.(lf.id, i)} title={labelOf(t.view)}>
            {t.view === "editor" && t.ref ? t.ref.split("/").pop() : labelOf(t.view)}
            <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
            {#if lf.tabs.length > 1}<span class="ptx" onpointerdown={(e) => e.stopPropagation()} onclick={(e) => { e.stopPropagation(); onCloseTab?.(lf.id, i); }}>×</span>{/if}
          </button>
        {/each}
        <button class="ptadd" title="New tab in pane" onclick={() => onAddTab?.(lf.id)}>+</button>
      </div>
      <select class="vpick" value={lf.view} onchange={(e) => onSetView(lf.id, (e.currentTarget as HTMLSelectElement).value as ViewKind)} title="Change this tab's view">
        {#each VIEWS as v (v.k)}<option value={v.k}>{v.label}</option>{/each}
      </select>
      <span class="sp"></span>
      <button class="pbtn" title="Split right" onclick={() => onSplit(lf.id, "right", lf.view, lf.ref)}>⊟</button>
      <button class="pbtn" title="Split down" onclick={() => onSplit(lf.id, "bottom", lf.view, lf.ref)}>⊞</button>
      <button class="pbtn close" title="Close pane" onclick={() => onClose(lf.id)}>✕</button>
    </div>
    {/if}
    <div class="pbody" role="group">
      {@render view(lf)}
      {#if dropHint && dropHint.leafId === lf.id}
        <div class="dropzone {dropHint.edge}"></div>
      {/if}
    </div>
  </div>
{/if}

<style>
  /* Direction modifier is `dir-row`/`dir-col`, NOT `row`/`col`: a bare `.row`
     collides with the global list-row utility in app.css (margin + align-items:
     center), which floated split panes inset and vertically-centered (#fill-bug). */
  .split { display: flex; width: 100%; height: 100%; align-items: stretch; }
  .split.dir-row { flex-direction: row; }
  .split.dir-col { flex-direction: column; }
  .cell { display: flex; flex-direction: column; position: relative; overflow: hidden; }
  .cell > :global(.leaf), .cell > :global(.split) { flex: 1 1 0%; min-width: 0; min-height: 0; }
  .divider { flex: 0 0 auto; background: var(--border); z-index: 3; }
  .divider.dir-row { width: 1px; cursor: col-resize; }
  .divider.dir-col { height: 1px; cursor: row-resize; }
  .divider.dir-row::after { content: ""; position: absolute; width: 7px; margin-left: -3px; top: 0; bottom: 0; cursor: col-resize; }
  .divider:hover { background: var(--accent); }

  /* Flush panes (no card look): no border/radius, no gap — panes meet at the
     1px .divider only. Active pane gets a subtle inset accent that adds no layout. */
  .leaf { display: flex; flex-direction: column; width: 100%; height: 100%; min-width: 0; min-height: 0;
    overflow: hidden; background: var(--bg);
    /* Pane enter (create/split): opacity + a hair of scale only — never animate
       width/height/flex (that reflows and reintroduces the floating-box look). */
    animation: pane-in 0.12s ease-out; }
  @keyframes pane-in { from { opacity: 0; } to { opacity: 1; } }
  @media (prefers-reduced-motion: reduce) { .leaf { animation: none; } }
  .leaf.dimmed > .pbody { opacity: 0.5; transition: opacity 0.15s ease; }
  .leaf.active { box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 55%, transparent); }
  .leaf.active > .phead { background: var(--sel); }
  .phead { display: flex; align-items: center; gap: 6px; height: 24px; flex: 0 0 auto; padding: 0 6px;
    background: var(--panel); border-bottom: 1px solid var(--border); cursor: grab; }
  .grip { color: var(--text3); font-size: 11px; }
  .ptabs { display: flex; align-items: center; gap: 2px; overflow-x: auto; scrollbar-width: none; min-width: 0; }
  .ptabs::-webkit-scrollbar { height: 0; }
  .ptab { display: inline-flex; align-items: center; gap: 4px; border: 0; background: transparent;
    color: var(--text3); font-family: var(--font-ui); font-size: 11px; padding: 2px 7px; border-radius: 5px;
    white-space: nowrap; cursor: default; max-width: 140px; overflow: hidden; text-overflow: ellipsis; }
  .ptab:hover { background: var(--panel2); color: var(--text2); }
  .ptab.on { background: var(--sel); color: var(--text); }
  .ptx { color: var(--text3); font-size: 12px; }
  .ptx:hover { color: var(--text); }
  .ptadd { border: 0; background: transparent; color: var(--text3); font-size: 13px; width: 18px; height: 18px;
    border-radius: 4px; cursor: default; flex: 0 0 auto; }
  .ptadd:hover { background: var(--sel); color: var(--text); }
  .vpick { background: transparent; border: 0; color: var(--text2); font-family: var(--font-ui);
    font-size: 11.5px; outline: 0; cursor: default; }
  .sp { flex: 1; }
  .pbtn { border: 0; background: transparent; color: var(--text3); font-size: 11px; width: 18px; height: 18px;
    border-radius: 4px; cursor: default; }
  .pbtn:hover { background: var(--sel); color: var(--text); }
  .pbtn.close:hover { color: var(--red); }
  .pbody { position: relative; flex: 1; min-height: 0; overflow: hidden; }
  .dropzone { position: absolute; background: color-mix(in srgb, var(--accent) 22%, transparent);
    border: 2px solid var(--accent); pointer-events: none; z-index: 5;
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 40%, transparent),
      0 0 12px color-mix(in srgb, var(--accent) 35%, transparent);
    animation: dropzone-in 0.09s ease-out; }
  @keyframes dropzone-in { from { opacity: 0; transform: scale(0.97); } to { opacity: 1; transform: none; } }
  @media (prefers-reduced-motion: reduce) { .dropzone { animation: none; } }
  .dropzone.left { left: 0; top: 0; bottom: 0; width: 50%; }
  .dropzone.right { right: 0; top: 0; bottom: 0; width: 50%; }
  .dropzone.top { left: 0; right: 0; top: 0; height: 50%; }
  .dropzone.bottom { left: 0; right: 0; bottom: 0; height: 50%; }
  .dropzone.center { inset: 12%; }
</style>
