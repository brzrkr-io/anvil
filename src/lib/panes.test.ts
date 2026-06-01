import { describe, it, expect } from "vitest";
import {
  leaf, splitLeaf, closeLeaf, resizeSplit, dockLeaf, setView,
  findLeaf, leafCount, firstLeaf, balanceTree, closeOthers, leafIds,
  addTab, setActiveTab, closeTab, type PaneNode, type SplitNode,
} from "./panes";

describe("tabs in panes", () => {
  it("a leaf starts with one tab mirroring its view/ref", () => {
    const lf = leaf("term", "t1", "A");
    expect(lf.tabs.length).toBe(1);
    expect(lf.tabs[0].view).toBe("term");
    expect(lf.view).toBe("term");
    expect(lf.ref).toBe("t1");
  });
  it("addTab appends + activates; mirror follows the active tab", () => {
    const tree = addTab(leaf("term", "t1", "A"), "A", "editor", "/f.ts");
    const lf = findLeaf(tree, "A")!;
    expect(lf.tabs.length).toBe(2);
    expect(lf.active).toBe(1);
    expect(lf.view).toBe("editor");
    expect(lf.ref).toBe("/f.ts");
  });
  it("setActiveTab switches the mirror back", () => {
    let tree = addTab(leaf("term", "t1", "A"), "A", "editor", "/f.ts");
    tree = setActiveTab(tree, "A", 0);
    expect(findLeaf(tree, "A")!.view).toBe("term");
  });
  it("closeTab keeps the pane while other tabs remain", () => {
    let tree = addTab(leaf("term", "t1", "A"), "A", "editor", "/f.ts");
    tree = closeTab(tree, "A", 1);
    const lf = findLeaf(tree, "A")!;
    expect(lf.tabs.length).toBe(1);
    expect(lf.view).toBe("term");
  });
  it("closeTab on the last tab closes the pane", () => {
    let tree: PaneNode = splitLeaf(leaf("term", "t1", "A"), "A", "right", "files").tree;
    tree = closeTab(tree, "A", 0);
    expect(findLeaf(tree, "A")).toBeNull();
  });
});

describe("tab edge cases", () => {
  it("closeTab on the active tab moves active left and keeps the mirror in sync", () => {
    let tree: PaneNode = addTab(addTab(leaf("term", "t1", "A"), "A", "editor", "/a.ts"), "A", "scm");
    expect(findLeaf(tree, "A")!.active).toBe(2); // scm active
    tree = closeTab(tree, "A", 2);
    const lf = findLeaf(tree, "A")!;
    expect(lf.tabs.length).toBe(2);
    expect(lf.active).toBe(1);
    expect(lf.view).toBe("editor");
  });
  it("addTab works on a leaf nested inside a split", () => {
    let tree: PaneNode = splitLeaf(leaf("term", "t1", "A"), "A", "right", "files").tree;
    tree = addTab(tree, "A", "agent");
    const lf = findLeaf(tree, "A")!;
    expect(lf.tabs.length).toBe(2);
    expect(lf.view).toBe("agent");
  });
  it("setActiveTab clamps out-of-range indices", () => {
    let tree = addTab(leaf("term", "t1", "A"), "A", "editor", "/a.ts");
    tree = setActiveTab(tree, "A", 99);
    expect(findLeaf(tree, "A")!.active).toBe(1); // clamped to last
  });
});

describe("balanceTree nested", () => {
  it("equalizes every split level", () => {
    let tree: PaneNode = splitLeaf(leaf("term", "t1", "A"), "A", "right", "files").tree; // row [A, B]
    const bId = leafIds(tree).find((id) => id !== "A")!;
    tree = splitLeaf(tree, bId, "bottom", "scm").tree; // B becomes a col split
    tree = resizeSplit(tree, (tree as SplitNode).id, 0, 0.3);
    const balanced = balanceTree(tree) as SplitNode;
    expect(balanced.sizes).toEqual([0.5, 0.5]);
    expect(leafCount(balanced)).toBe(3);
  });
});

describe("closeOthers", () => {
  it("collapses the tree to just the kept leaf", () => {
    let tree: PaneNode = leaf("term", "t1", "A");
    tree = splitLeaf(tree, "A", "right", "files").tree;
    const bId = leafIds(tree).find((id) => id !== "A")!;
    tree = splitLeaf(tree, bId, "bottom", "scm").tree;
    expect(leafCount(tree)).toBe(3);
    const only = closeOthers(tree, "A");
    expect(leafCount(only)).toBe(1);
    expect(findLeaf(only, "A")).toBeTruthy();
  });
});

describe("balanceTree", () => {
  it("resets all split sizes to equal fractions", () => {
    let tree: PaneNode = leaf("term", "t1", "A");
    tree = splitLeaf(tree, "A", "right", "files").tree;
    tree = resizeSplit(tree, (tree as SplitNode).id, 0, 0.2);
    expect((tree as SplitNode).sizes).not.toEqual([0.5, 0.5]);
    const balanced = balanceTree(tree) as SplitNode;
    expect(balanced.sizes).toEqual([0.5, 0.5]);
  });
});

describe("splitLeaf", () => {
  it("wraps a lone leaf in a split with the new pane on the requested edge", () => {
    const root = leaf("term", "t1", "A");
    const { tree, newLeafId } = splitLeaf(root, "A", "right", "files");
    expect(tree.kind).toBe("split");
    const s = tree as SplitNode;
    expect(s.dir).toBe("row");
    expect(s.children.map((c) => (c as any).id)).toEqual(["A", newLeafId]); // right = after
    expect(s.sizes).toEqual([0.5, 0.5]);
  });

  it("inserts inline when the parent split already matches direction", () => {
    let tree: PaneNode = leaf("term", "t1", "A");
    tree = splitLeaf(tree, "A", "right", "files").tree; // row [A, B]
    const bId = (tree as SplitNode).children[1].id;
    tree = splitLeaf(tree, bId, "right", "scm").tree; // should stay one row of 3
    expect(tree.kind).toBe("split");
    expect((tree as SplitNode).children).toHaveLength(3);
    expect((tree as SplitNode).sizes).toEqual([1 / 3, 1 / 3, 1 / 3]);
  });

  it("nests a new split when direction differs", () => {
    let tree: PaneNode = leaf("term", "t1", "A");
    tree = splitLeaf(tree, "A", "right", "files").tree; // row [A, B]
    tree = splitLeaf(tree, "A", "bottom", "scm").tree; // A becomes a col split
    const top = tree as SplitNode;
    expect(top.dir).toBe("row");
    expect(top.children[0].kind).toBe("split");
    expect((top.children[0] as SplitNode).dir).toBe("col");
  });
});

describe("closeLeaf", () => {
  it("collapses a split back to a single leaf when one child remains", () => {
    let tree: PaneNode = leaf("term", "t1", "A");
    tree = splitLeaf(tree, "A", "right", "files").tree;
    const bId = (tree as SplitNode).children[1].id;
    tree = closeLeaf(tree, bId);
    expect(tree.kind).toBe("leaf");
    expect((tree as any).id).toBe("A");
  });

  it("refuses to remove the only pane", () => {
    const root = leaf("term", "t1", "A");
    expect(closeLeaf(root, "A")).toBe(root);
  });
});

describe("resizeSplit", () => {
  it("shifts the boundary and conserves total size", () => {
    let tree: PaneNode = leaf("term", undefined, "A");
    const r = splitLeaf(tree, "A", "right", "files");
    tree = r.tree;
    const sid = (tree as SplitNode).id;
    tree = resizeSplit(tree, sid, 0, 0.2);
    const s = tree as SplitNode;
    expect(s.sizes[0]).toBeCloseTo(0.7);
    expect(s.sizes[1]).toBeCloseTo(0.3);
    expect(s.sizes[0] + s.sizes[1]).toBeCloseTo(1);
  });

  it("clamps so a pane never collapses below the minimum", () => {
    let tree: PaneNode = leaf("term", undefined, "A");
    tree = splitLeaf(tree, "A", "right", "files").tree;
    const sid = (tree as SplitNode).id;
    tree = resizeSplit(tree, sid, 0, 0.99);
    const s = tree as SplitNode;
    expect(s.sizes[1]).toBeGreaterThanOrEqual(0.08);
  });
});

describe("dockLeaf", () => {
  it("moves a pane to a new edge of the target", () => {
    let tree: PaneNode = leaf("term", undefined, "A");
    tree = splitLeaf(tree, "A", "right", "files").tree; // row [A(term), B(files)]
    const bId = (tree as SplitNode).children[1].id;
    // dock B under A → A becomes a col split [A, B], top-level collapses to it
    tree = dockLeaf(tree, bId, "A", "bottom");
    expect(leafCount(tree)).toBe(2);
    expect(tree.kind).toBe("split");
    expect((tree as SplitNode).dir).toBe("col");
  });

  it("center-dock swaps the target view and removes the dragged pane", () => {
    let tree: PaneNode = leaf("term", undefined, "A");
    tree = splitLeaf(tree, "A", "right", "agent").tree;
    const bId = (tree as SplitNode).children[1].id;
    tree = dockLeaf(tree, bId, "A", "center");
    expect(tree.kind).toBe("leaf");
    expect((tree as any).view).toBe("agent");
  });
});

describe("setView / helpers", () => {
  it("setView changes only the targeted leaf", () => {
    let tree: PaneNode = leaf("term", undefined, "A");
    tree = splitLeaf(tree, "A", "right", "files").tree;
    tree = setView(tree, "A", "scm");
    expect((findLeaf(tree, "A") as any).view).toBe("scm");
  });
  it("firstLeaf descends to the leftmost leaf", () => {
    let tree: PaneNode = leaf("term", undefined, "A");
    tree = splitLeaf(tree, "A", "right", "files").tree;
    expect(firstLeaf(tree).id).toBe("A");
  });
});
