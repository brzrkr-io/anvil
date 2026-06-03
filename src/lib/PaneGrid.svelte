<script lang="ts">
  import { leafIds, type PaneNode, type SplitNode, type Leaf, type ViewKind, type Edge } from "./panes";
  import PaneGrid from "./PaneGrid.svelte";
  import type { Snippet } from "svelte";

  let {
    node,
    view,
    drag,
    activeId = "",
    onSplit,
    onClose,
    onSetView,
    onResize,
    onDock,
    onFocusLeaf,
    onDragStart,
    onDragEnd,
    extDrag = null,
    onDropExternal,
    onSetActiveTab,
    onCloseTab,
    onAddTab,
    zoomId = null,
    dim = false,
    depth = 0,
  }: {
    node: PaneNode;
    view: Snippet<[Leaf]>;
    drag: { id: string | null };
    activeId?: string;
    onSplit: (leafId: string, edge: Edge, v: ViewKind, srcRef?: string) => void;
    onClose: (leafId: string) => void;
    onSetView: (leafId: string, v: ViewKind) => void;
    onResize: (splitId: string, index: number, deltaFrac: number) => void;
    onDock: (dragId: string, targetId: string, edge: Edge) => void;
    onFocusLeaf?: (leafId: string) => void;
    onDragStart: (leafId: string) => void;
    onDragEnd: () => void;
    // A tab dragged from the top strip into a pane quadrant (#4/#5).
    extDrag?: { view: ViewKind; ref?: string } | null;
    onDropExternal?: (targetLeafId: string, edge: Edge) => void;
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

  // ── dock drop zones ──
  let hoverEdge = $state<Edge | null>(null);
  function edgeFromPointer(el: HTMLElement, e: MouseEvent): Edge {
    const r = el.getBoundingClientRect();
    const x = (e.clientX - r.left) / r.width;
    const y = (e.clientY - r.top) / r.height;
    const m = 0.28;
    if (x < m) return "left";
    if (x > 1 - m) return "right";
    if (y < m) return "top";
    if (y > 1 - m) return "bottom";
    return "center";
  }
</script>

{#if node.kind === "split"}
  {@const sp = node}
  <div class="split {sp.dir}" bind:this={splitEl}>
    {#each sp.children as child, i (child.id)}
      {@const hide = zoomId && !leafIds(child).includes(zoomId)}
      <div class="cell" style="flex: {hide ? 0 : zoomId ? 1 : sp.sizes[i]} 1 0; min-width:0; min-height:0; {hide ? 'display:none;' : ''}">
        <PaneGrid node={child} {view} {drag} {activeId} {onSplit} {onClose} {onSetView} {onResize} {onDock} {onFocusLeaf} {onDragStart} {onDragEnd} {extDrag} {onDropExternal} {onSetActiveTab} {onCloseTab} {onAddTab} {zoomId} {dim} depth={depth + 1} />
      </div>
      {#if i < sp.children.length - 1 && !zoomId}
        <div class="divider {sp.dir}" onpointerdown={(e) => startResize(e, sp, i)} role="separator" tabindex="-1"></div>
      {/if}
    {/each}
  </div>
{:else}
  {@const lf = node}
  <div class="leaf {lf.id === activeId ? 'active' : ''}" class:dimmed={dim && lf.id !== activeId} onpointerdowncapture={() => onFocusLeaf?.(lf.id)}>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="phead"
      draggable="true"
      ondragstart={(e) => { onDragStart(lf.id); if (e.dataTransfer) { e.dataTransfer.setData('text/plain', lf.id); e.dataTransfer.effectAllowed = 'move'; } }}
      ondragend={onDragEnd}
    >
      <span class="grip">⠿</span>
      <div class="ptabs">
        {#each lf.tabs as t, i (t.id)}
          <button class="ptab {i === lf.active ? 'on' : ''}" onclick={() => onSetActiveTab?.(lf.id, i)} title={labelOf(t.view)}>
            {t.view === "editor" && t.ref ? t.ref.split("/").pop() : labelOf(t.view)}
            <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
            {#if lf.tabs.length > 1}<span class="ptx" onclick={(e) => { e.stopPropagation(); onCloseTab?.(lf.id, i); }}>×</span>{/if}
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
    <div
      class="pbody"
      role="group"
      ondragover={(e) => { if ((drag.id && drag.id !== lf.id) || extDrag) { e.preventDefault(); hoverEdge = edgeFromPointer(e.currentTarget as HTMLElement, e); } }}
      ondragleave={() => (hoverEdge = null)}
      ondrop={(e) => {
        if (hoverEdge) {
          if (extDrag) { e.preventDefault(); onDropExternal?.(lf.id, hoverEdge); }
          else if (drag.id && drag.id !== lf.id) { e.preventDefault(); onDock(drag.id, lf.id, hoverEdge); }
        }
        hoverEdge = null;
      }}
    >
      {@render view(lf)}
      {#if ((drag.id && drag.id !== lf.id) || extDrag) && hoverEdge}
        <div class="dropzone {hoverEdge}"></div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .split { display: flex; width: 100%; height: 100%; }
  .split.row { flex-direction: row; }
  .split.col { flex-direction: column; }
  /* Column flex so the pane child fills via flex-grow, not height:100% (which
     WebKit fails to resolve against a flex-stretched parent → the pane floats
     vertically-inset with open space). */
  .cell { display: flex; flex-direction: column; position: relative; overflow: hidden; }
  .cell > :global(.leaf), .cell > :global(.split) { flex: 1 1 0%; min-width: 0; min-height: 0; height: auto; }
  .divider { flex: 0 0 auto; background: var(--border); z-index: 3; }
  .divider.row { width: 1px; cursor: col-resize; }
  .divider.col { height: 1px; cursor: row-resize; }
  .divider.row::after { content: ""; position: absolute; width: 7px; margin-left: -3px; top: 0; bottom: 0; cursor: col-resize; }
  .divider:hover { background: var(--accent); }

  .leaf { display: flex; flex-direction: column; width: 100%; height: 100%; min-width: 0; min-height: 0;
    border: 1px solid var(--border); border-radius: 6px; overflow: hidden; background: var(--bg); }
  .leaf.dimmed > .pbody { opacity: 0.5; transition: opacity 0.15s ease; }
  .leaf.active { border-color: var(--accent); }
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
  .dropzone { position: absolute; background: color-mix(in srgb, var(--accent) 30%, transparent);
    border: 1px solid var(--accent); pointer-events: none; z-index: 5; }
  .dropzone.left { left: 0; top: 0; bottom: 0; width: 50%; }
  .dropzone.right { right: 0; top: 0; bottom: 0; width: 50%; }
  .dropzone.top { left: 0; right: 0; top: 0; height: 50%; }
  .dropzone.bottom { left: 0; right: 0; bottom: 0; height: 50%; }
  .dropzone.center { inset: 12%; }
</style>
