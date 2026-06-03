// Pointer-based tab drag (IDE-grade drag-to-split). The DOM glue — window
// listeners, the floating ghost, hit-testing — lives in +page.svelte; the pure,
// testable bits live here: the edge math and the drop decision. HTML5
// drag-and-drop is unreliable in the app's WebView (drops silently no-op), so
// the whole flow is pointerdown/move/up + elementFromPoint hit-testing.

import type { Edge, ViewKind } from "./panes";

/** What is being dragged. `from` is set when the source is an existing pane tab
 *  (so a drop can MOVE it), absent when dragged from the top tab strip. */
export interface TabDrag {
  view: ViewKind;
  ref?: string;
  label: string;
  from?: { leafId: string; index: number };
}

/** Pointer moved more than this (px) from the start → it's a drag, not a click.
 *  Below the threshold the gesture still selects/opens the tab. */
export const DRAG_THRESHOLD = 5;

/** Fraction of a pane's side that counts as an edge drop zone; inside all four
 *  margins is the center (add-as-tab) zone. */
export const EDGE_RATIO = 0.28;

/** Which drop edge a pointer at (x,y) maps to within `rect`. Closer-than-ratio
 *  to a side wins (left/right before top/bottom); otherwise center. */
export function edgeFromRect(rect: { left: number; top: number; width: number; height: number }, x: number, y: number): Edge {
  const fx = (x - rect.left) / rect.width;
  const fy = (y - rect.top) / rect.height;
  if (fx < EDGE_RATIO) return "left";
  if (fx > 1 - EDGE_RATIO) return "right";
  if (fy < EDGE_RATIO) return "top";
  if (fy > 1 - EDGE_RATIO) return "bottom";
  return "center";
}

/** True once the pointer has travelled past the click/drag threshold. */
export function passedThreshold(startX: number, startY: number, x: number, y: number): boolean {
  return Math.abs(x - startX) > DRAG_THRESHOLD || Math.abs(y - startY) > DRAG_THRESHOLD;
}

export type DropAction =
  | { kind: "none" }
  | { kind: "addTab"; leafId: string }
  | { kind: "moveTab"; from: { leafId: string; index: number }; to: string }
  | { kind: "split"; leafId: string; edge: Edge }
  | { kind: "splitFrom"; from: { leafId: string; index: number }; leafId: string; edge: Edge };

/**
 * Resolve a drop into a concrete tree operation. `hint` is the pane+edge under
 * the cursor at release (null → no valid target, caller treats as a click).
 *  - center, top-strip source        → add as a tab on the target pane.
 *  - center, pane-tab source          → move that tab into the target pane.
 *  - edge,   top-strip source         → split the target pane.
 *  - edge,   pane-tab source          → split the target with the tab, drop the source.
 * Dropping a pane tab back onto its own pane is a no-op (nothing to do).
 */
export function dropAction(drag: TabDrag, hint: { leafId: string; edge: Edge } | null): DropAction {
  if (!hint) return { kind: "none" };
  const { leafId, edge } = hint;
  if (drag.from && drag.from.leafId === leafId) return { kind: "none" };
  if (edge === "center") {
    return drag.from
      ? { kind: "moveTab", from: drag.from, to: leafId }
      : { kind: "addTab", leafId };
  }
  return drag.from
    ? { kind: "splitFrom", from: drag.from, leafId, edge }
    : { kind: "split", leafId, edge };
}
