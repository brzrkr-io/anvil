<script lang="ts">
  // A draggable divider between two panes. Bind `size` to the controlled pane's
  // flex-basis (px); the resizer updates it on drag, clamps, and persists.
  //
  //   <div style="flex:0 0 {w}px">…left…</div>
  //   <Resizer bind:size={w} min={200} max={700} storeKey="my-pane" />
  //   <div style="flex:1">…right…</div>
  //
  // `edge` says which side of the divider the controlled pane is on:
  //   left/top  → the pane BEFORE the divider (default)
  //   right/bottom → the pane AFTER the divider (drag direction inverts)
  let {
    size = $bindable(),
    min = 160,
    max = 900,
    edge = "left",
    storeKey = "",
  }: { size: number; min?: number; max?: number; edge?: "left" | "right" | "top" | "bottom"; storeKey?: string } = $props();

  const vertical = $derived(edge === "top" || edge === "bottom"); // divider drags up/down
  let dragging = $state(false);
  let startPos = 0;
  let startSize = 0;

  function onMove(e: PointerEvent) {
    const pos = vertical ? e.clientY : e.clientX;
    let delta = pos - startPos;
    if (edge === "right" || edge === "bottom") delta = -delta;
    size = Math.max(min, Math.min(max, startSize + delta));
  }
  function onUp() {
    dragging = false;
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", onUp);
    if (storeKey) { try { localStorage.setItem(storeKey, String(Math.round(size))); } catch { /* ignore */ } }
  }
  function onDown(e: PointerEvent) {
    dragging = true;
    startPos = vertical ? e.clientY : e.clientX;
    startSize = size;
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    e.preventDefault();
  }
</script>

<div
  class="rsz {vertical ? 'v' : 'h'}"
  class:on={dragging}
  role="separator"
  tabindex="-1"
  aria-orientation={vertical ? "horizontal" : "vertical"}
  onpointerdown={onDown}
></div>

<style>
  .rsz {
    position: relative;
    z-index: 5;
    flex: 0 0 5px;
    align-self: stretch;
    touch-action: none;
  }
  .rsz.h { cursor: col-resize; }
  .rsz.v { cursor: row-resize; align-self: auto; width: 100%; height: 5px; flex: 0 0 5px; }
  .rsz::before {
    content: "";
    position: absolute;
    background: var(--border);
    transition: background 0.12s;
  }
  .rsz.h::before { top: 0; bottom: 0; left: 2px; width: 1px; }
  .rsz.v::before { left: 0; right: 0; top: 2px; height: 1px; }
  .rsz:hover::before, .rsz.on::before { background: var(--accent); }
  .rsz.h:hover::before, .rsz.h.on::before { width: 2px; }
  .rsz.v:hover::before, .rsz.v.on::before { height: 2px; }
</style>
