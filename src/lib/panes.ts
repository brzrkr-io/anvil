// Dockable pane-tree model. A workspace is a binary-ish tree: leaves hold a
// view (terminal/editor/files/…), splits arrange children in a row or column
// with fractional sizes. All ops are pure and return a new tree so the renderer
// can diff cleanly and the model stays unit-testable.

export type ViewKind =
  | "term" | "editor" | "files" | "scm" | "search"
  | "agent" | "devops" | "settings" | "welcome";

export type Dir = "row" | "col"; // row = side-by-side, col = stacked
export type Edge = "left" | "right" | "top" | "bottom" | "center";

/** One tab inside a pane. */
export interface PaneTab {
  id: string;
  view: ViewKind;
  ref?: string;
}
export interface Leaf {
  kind: "leaf";
  id: string;
  /** `view`/`ref` mirror the active tab so existing readers keep working. */
  view: ViewKind;
  /** opaque per-view payload, e.g. terminal session id or file path */
  ref?: string;
  tabs: PaneTab[];
  active: number;
}
export interface SplitNode {
  kind: "split";
  id: string;
  dir: Dir;
  children: PaneNode[];
  sizes: number[]; // fractions, sum ≈ 1, len == children.length
}
export type PaneNode = Leaf | SplitNode;

let _seq = 0;
export function paneId(prefix = "p"): string {
  _seq += 1;
  return `${prefix}${_seq}`;
}

export function leaf(view: ViewKind, ref?: string, id = paneId("l")): Leaf {
  return { kind: "leaf", id, view, ref, tabs: [{ id: paneId("tab"), view, ref }], active: 0 };
}

/** Keep a leaf's `view`/`ref` mirror in step with its active tab. */
function syncTabs(lf: Leaf, tabs: PaneTab[], active: number): Leaf {
  const a = Math.max(0, Math.min(active, tabs.length - 1));
  return { ...lf, tabs, active: a, view: tabs[a].view, ref: tabs[a].ref };
}

/** Add a tab to a pane and make it active (#2). */
export function addTab(root: PaneNode, leafId: string, view: ViewKind, ref?: string): PaneNode {
  return mapTree(root, (n) => {
    if (n.kind !== "leaf" || n.id !== leafId) return n;
    const tabs = [...n.tabs, { id: paneId("tab"), view, ref }];
    return syncTabs(n, tabs, tabs.length - 1);
  });
}

/** Switch the active tab of a pane. */
export function setActiveTab(root: PaneNode, leafId: string, idx: number): PaneNode {
  return mapTree(root, (n) => (n.kind === "leaf" && n.id === leafId ? syncTabs(n, n.tabs, idx) : n));
}

/** Close a tab. Closing the pane's last tab closes the pane. */
export function closeTab(root: PaneNode, leafId: string, idx: number): PaneNode {
  const lf = findLeaf(root, leafId);
  if (!lf) return root;
  if (lf.tabs.length <= 1) return closeLeaf(root, leafId);
  return mapTree(root, (n) => {
    if (n.kind !== "leaf" || n.id !== leafId) return n;
    const tabs = n.tabs.filter((_, i) => i !== idx);
    const active = n.active > idx ? n.active - 1 : n.active;
    return syncTabs(n, tabs, active);
  });
}

function normalizeSizes(n: number): number[] {
  return Array.from({ length: n }, () => 1 / n);
}

/** Find a leaf by id (depth-first). */
export function findLeaf(node: PaneNode, id: string): Leaf | null {
  if (node.kind === "leaf") return node.id === id ? node : null;
  for (const c of node.children) {
    const f = findLeaf(c, id);
    if (f) return f;
  }
  return null;
}

export function firstLeaf(node: PaneNode): Leaf {
  let n: PaneNode = node;
  while (n.kind === "split") n = n.children[0];
  return n;
}

export function leafCount(node: PaneNode): number {
  return node.kind === "leaf" ? 1 : node.children.reduce((a, c) => a + leafCount(c), 0);
}

/** Give every terminal leaf a fresh ref (pty id). Used when restoring a saved
 *  layout, since the old terminal sessions are gone. */
export function remapTermRefs(node: PaneNode): PaneNode {
  if (node.kind === "leaf") {
    // Migrate older saved trees that predate per-pane tabs.
    const tabs: PaneTab[] = node.tabs ?? [{ id: paneId("tab"), view: node.view, ref: node.ref }];
    const remapped = tabs.map((t) => (t.view === "term" ? { ...t, ref: paneId("wt") } : t));
    return syncTabs({ ...node, tabs: remapped, active: node.active ?? 0 }, remapped, node.active ?? 0);
  }
  return { ...node, children: node.children.map(remapTermRefs) };
}

/** Map a transform over the tree, returning a new tree (structural sharing). */
function mapTree(node: PaneNode, fn: (n: PaneNode) => PaneNode): PaneNode {
  if (node.kind === "leaf") return fn(node);
  const children = node.children.map((c) => mapTree(c, fn));
  return fn({ ...node, children });
}

/** Replace the active tab's view (and optional ref). */
export function setView(root: PaneNode, leafId: string, view: ViewKind, ref?: string): PaneNode {
  return mapTree(root, (n) => {
    if (n.kind !== "leaf" || n.id !== leafId) return n;
    const tabs = n.tabs.map((t, i) => (i === n.active ? { ...t, view, ref } : t));
    return syncTabs(n, tabs, n.active);
  });
}

const dirForEdge = (edge: Edge): Dir => (edge === "left" || edge === "right" ? "row" : "col");
const before = (edge: Edge): boolean => edge === "left" || edge === "top";

/**
 * Split `targetId` along `edge`, inserting a new leaf showing `view`. If the
 * target's parent already runs in the needed direction, the new pane joins that
 * split (keeps the tree flat); otherwise the leaf is wrapped in a new split.
 */
export function splitLeaf(
  root: PaneNode,
  targetId: string,
  edge: Edge,
  view: ViewKind,
  ref?: string,
): { tree: PaneNode; newLeafId: string } {
  const dir = dirForEdge(edge);
  const insertBefore = before(edge);
  const fresh = leaf(view, ref);

  function rec(node: PaneNode): PaneNode {
    if (node.kind === "leaf") {
      if (node.id !== targetId) return node;
      const kids = insertBefore ? [fresh, node] : [node, fresh];
      return { kind: "split", id: paneId("s"), dir, children: kids, sizes: normalizeSizes(2) };
    }
    // If this split holds the target directly and matches dir, insert inline.
    if (node.dir === dir) {
      const idx = node.children.findIndex((c) => c.kind === "leaf" && c.id === targetId);
      if (idx >= 0) {
        const at = insertBefore ? idx : idx + 1;
        const children = [...node.children];
        children.splice(at, 0, fresh);
        return { ...node, children, sizes: normalizeSizes(children.length) };
      }
    }
    return { ...node, children: node.children.map(rec) };
  }
  return { tree: rec(root), newLeafId: fresh.id };
}

/** Remove a leaf; collapse any split left with a single child. Returns the
 *  (possibly unchanged) tree — never returns null even for the last leaf. */
export function closeLeaf(root: PaneNode, leafId: string): PaneNode {
  if (root.kind === "leaf") return root; // can't remove the only pane
  function rec(node: SplitNode): PaneNode {
    const kept: PaneNode[] = [];
    for (const c of node.children) {
      if (c.kind === "leaf") {
        if (c.id !== leafId) kept.push(c);
      } else {
        const r = rec(c);
        kept.push(r);
      }
    }
    if (kept.length === 1) return kept[0]; // collapse
    return { ...node, children: kept, sizes: normalizeSizes(kept.length) };
  }
  const out = rec(root);
  return out;
}

/** Adjust the boundary between children[index] and children[index+1] of a split
 *  by `deltaFrac` (fraction of the split's main axis). Clamped to keep panes
 *  from collapsing. */
export function resizeSplit(root: PaneNode, splitId: string, index: number, deltaFrac: number): PaneNode {
  const MIN = 0.08;
  return mapTree(root, (n) => {
    if (n.kind !== "split" || n.id !== splitId) return n;
    if (index < 0 || index >= n.sizes.length - 1) return n;
    const sizes = [...n.sizes];
    let a = sizes[index] + deltaFrac;
    let b = sizes[index + 1] - deltaFrac;
    if (a < MIN) { b -= MIN - a; a = MIN; }
    if (b < MIN) { a -= MIN - b; b = MIN; }
    sizes[index] = a;
    sizes[index + 1] = b;
    return { ...n, sizes };
  });
}

/**
 * Move the leaf `dragId` to dock onto `targetId` at `edge`. center = swap the
 * target's view with the dragged one (tab-style replace). Removes the dragged
 * leaf from its old spot first, then splits the target.
 */
export function dockLeaf(root: PaneNode, dragId: string, targetId: string, edge: Edge): PaneNode {
  if (dragId === targetId) return root;
  const dragged = findLeaf(root, dragId);
  if (!dragged) return root;
  if (edge === "center") {
    // Replace target's view with dragged view, then remove the dragged leaf.
    const swapped = setView(root, targetId, dragged.view, dragged.ref);
    return closeLeaf(swapped, dragId);
  }
  const without = closeLeaf(root, dragId);
  // target may have been collapsed away if it shared a split with dragged —
  // findLeaf guards that.
  if (!findLeaf(without, targetId)) return root;
  const { tree } = splitLeaf(without, targetId, edge, dragged.view, dragged.ref);
  return tree;
}

/** Reset every split in the tree to equal-sized children (balance command, #10). */
export function balanceTree(root: PaneNode): PaneNode {
  if (root.kind === "leaf") return root;
  return {
    ...root,
    children: root.children.map(balanceTree),
    sizes: normalizeSizes(root.children.length),
  };
}

/** All leaf ids in document order. */
export function leafIds(node: PaneNode): string[] {
  if (node.kind === "leaf") return [node.id];
  return node.children.flatMap(leafIds);
}

/** Close every pane except `keepId`, collapsing the tree to that single leaf (#11). */
export function closeOthers(root: PaneNode, keepId: string): PaneNode {
  if (!findLeaf(root, keepId)) return root;
  let tree = root;
  for (const id of leafIds(root)) {
    if (id !== keepId) tree = closeLeaf(tree, id);
  }
  return tree;
}
