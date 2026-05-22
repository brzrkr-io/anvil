//! Pane-tree layout engine — pure geometry, no AppKit / Metal / PTY.
//!
//! The caller owns a `PaneTree` that describes how the window is split into
//! leaf panes. Each leaf is identified by a `PaneId` (a stable handle into a
//! pane registry built in a later phase). The tree drives:
//!   - pixel-rect assignment (`layout`)
//!   - mouse hit-testing (`hitTest`)
//!   - directional keyboard navigation (`neighbor`)
//!
//! ## Layout output
//!
//! `layout` writes into a caller-owned `std.ArrayListUnmanaged(LayoutEntry)`.
//! The caller clears the list before each call and pre-allocates sufficient
//! capacity to avoid allocations inside `layout`. This makes the call
//! allocation-free in steady state.
//!
//! ## Tree invariants
//!
//! - The root is never null; there is always >= 1 leaf.
//! - Every `Split` has >= 2 children.
//! - `ratios` sums to 1.0 (within floating-point error) for every split.
//! - `focused` always names a leaf that exists in the tree.

const std = @import("std");

/// A rectangle in device pixels. y=0 at the top (raster space).
pub const Rect = struct {
    x: f64,
    y: f64,
    w: f64,
    h: f64,

    /// True when `px, py` falls inside (or on the left/top boundary of) this rect.
    pub fn contains(self: Rect, px: f64, py: f64) bool {
        return px >= self.x and px < self.x + self.w and
            py >= self.y and py < self.y + self.h;
    }

    pub fn centerX(self: Rect) f64 {
        return self.x + self.w * 0.5;
    }

    pub fn centerY(self: Rect) f64 {
        return self.y + self.h * 0.5;
    }
};

pub const SplitDir = enum {
    /// Children laid out left | right — a vertical divider line between them.
    horizontal,
    /// Children laid out top | bottom — a horizontal divider line between them.
    vertical,
};

/// Stable handle into a pane registry (registry built in a later phase).
pub const PaneId = u32;

/// One node of the tree — a tagged union: leaf or split.
pub const PaneNode = union(enum) {
    leaf: PaneId,
    split: Split,
};

pub const Split = struct {
    dir: SplitDir,
    /// Children in visual order, heap-owned. Length >= 2.
    children: std.ArrayListUnmanaged(*PaneNode),
    /// One ratio per child; sums to 1.0.
    ratios: std.ArrayListUnmanaged(f64),
};

pub const PaneTree = struct {
    alloc: std.mem.Allocator,
    root: *PaneNode,
    focused: PaneId,
    /// True after closeLeaf empties the tree (root is invalid; only deinit is safe).
    empty: bool = false,

    /// Create a tree with a single leaf, focused on it.
    pub fn initSingle(alloc: std.mem.Allocator, first: PaneId) !PaneTree {
        const node = try alloc.create(PaneNode);
        node.* = .{ .leaf = first };
        return .{
            .alloc = alloc,
            .root = node,
            .focused = first,
        };
    }

    /// Free every `*PaneNode` recursively and all ArrayLists.
    pub fn deinit(self: *PaneTree) void {
        if (!self.empty) freeNode(self.alloc, self.root);
    }

    /// Split the FOCUSED leaf, inserting `new` beside it.
    ///
    /// Flat-split rule: if the focused leaf's immediate parent split already
    /// has the same `dir`, insert `new` as a sibling (no nesting). Otherwise
    /// wrap the focused leaf in a new two-child split.
    ///
    /// Ratio rule:
    ///  - New two-child split: each child gets 0.5.
    ///  - Sibling insert: the new child gets an equal share; all ratios are
    ///    renormalized so they sum to 1.0.
    pub fn split(self: *PaneTree, dir: SplitDir, new: PaneId) !void {
        const new_node = try self.alloc.create(PaneNode);
        errdefer self.alloc.destroy(new_node);
        new_node.* = .{ .leaf = new };

        const ctx = findIn(self.root, self.focused, null) orelse
            @panic("focused PaneId not found in tree");
        const focused_node = ctx.node;

        // Flat-split: parent already has the same direction — insert as sibling.
        if (ctx.parent) |par| {
            if (par.split.dir == dir) {
                const idx = findChildIndex(&par.split, focused_node);
                try insertSibling(&par.split, self.alloc, new_node, idx + 1);
                self.focused = new;
                return;
            }
        }

        // Wrap: build a new two-child split around the focused leaf.
        //
        // IMPORTANT: if the focused leaf IS the root we cannot put `root` itself
        // into children[0] — after we overwrite root.* the pointer would point
        // back to the split (cycle). Instead we allocate a new node to carry the
        // old leaf content, and overwrite root.* in place with the split.
        if (ctx.parent == null) {
            // focused_node == self.root. Allocate a fresh node for the old leaf.
            const old_leaf = try self.alloc.create(PaneNode);
            errdefer self.alloc.destroy(old_leaf);
            old_leaf.* = .{ .leaf = self.focused };

            var children = std.ArrayListUnmanaged(*PaneNode).empty;
            errdefer children.deinit(self.alloc);
            var ratios = std.ArrayListUnmanaged(f64).empty;
            errdefer ratios.deinit(self.alloc);

            try children.append(self.alloc, old_leaf);
            try children.append(self.alloc, new_node);
            try ratios.append(self.alloc, 0.5);
            try ratios.append(self.alloc, 0.5);

            // Overwrite root in place — the *PaneNode pointer stays the same.
            self.root.* = .{ .split = .{ .dir = dir, .children = children, .ratios = ratios } };
            self.focused = new;
            return;
        }

        // Non-root focused leaf: replace it in its parent with a new split node.
        const split_node = try self.alloc.create(PaneNode);
        errdefer self.alloc.destroy(split_node);

        var children = std.ArrayListUnmanaged(*PaneNode).empty;
        errdefer children.deinit(self.alloc);
        var ratios = std.ArrayListUnmanaged(f64).empty;
        errdefer ratios.deinit(self.alloc);

        try children.append(self.alloc, focused_node);
        try children.append(self.alloc, new_node);
        try ratios.append(self.alloc, 0.5);
        try ratios.append(self.alloc, 0.5);

        split_node.* = .{ .split = .{ .dir = dir, .children = children, .ratios = ratios } };
        replaceInSplit(&ctx.parent.?.split, focused_node, split_node);
        self.focused = new;
    }

    /// Remove the leaf with `id`. Collapses single-child splits.
    /// Returns the PaneId that should receive focus next, or null if the tree
    /// is now empty. Never leaves `focused` pointing at the removed leaf.
    pub fn closeLeaf(self: *PaneTree, id: PaneId) ?PaneId {
        // Special case: the only leaf is the root itself.
        if (self.root.* == .leaf and self.root.leaf == id) {
            freeNode(self.alloc, self.root);
            self.empty = true;
            self.focused = 0;
            return null;
        }

        // Find the leaf and its parent (parent is guaranteed non-null here since
        // a lone root was handled above).
        const ctx = findIn(self.root, id, null) orelse return null;
        const parent_node = ctx.parent orelse return null;
        const sp = &parent_node.split;

        // Choose the sibling that will receive focus next.
        const rm_idx = findChildIndex(sp, ctx.node);
        const next_focus_node: *PaneNode = if (rm_idx + 1 < sp.children.items.len)
            sp.children.items[rm_idx + 1]
        else
            sp.children.items[rm_idx - 1]; // rm_idx >= 1 since len >= 2
        const next_focus_id = firstLeafId(next_focus_node);

        // Remove the target child.
        freeNode(self.alloc, ctx.node);
        _ = sp.children.orderedRemove(rm_idx);
        _ = sp.ratios.orderedRemove(rm_idx);
        renormalize(sp.ratios.items);

        // Collapse: if the parent split now has only one child, replace it with
        // that child in the grandparent (or make it the new root).
        if (sp.children.items.len == 1) {
            const surviving = sp.children.items[0];
            collapseParent(self, parent_node, surviving);
        }

        self.focused = next_focus_id;
        return next_focus_id;
    }

    /// A single entry in the layout output.
    pub const LayoutEntry = struct { id: PaneId, rect: Rect };

    /// Recompute every leaf's pixel rect, writing results into `out`.
    /// `out` is cleared before use. The caller should pre-allocate capacity
    /// on `out` (e.g., via `ensureTotalCapacity`) to keep this allocation-free
    /// in steady state.
    pub fn layout(
        self: *const PaneTree,
        outer: Rect,
        divider_px: f64,
        out: *std.ArrayListUnmanaged(LayoutEntry),
        alloc: std.mem.Allocator,
    ) void {
        out.clearRetainingCapacity();
        layoutNode(self.root, outer, divider_px, out, alloc);
    }

    /// The leaf whose Rect contains the point `px, py`, or null if the point
    /// lands in a gutter (or outside `outer`).
    pub fn hitTest(
        self: *const PaneTree,
        outer: Rect,
        divider_px: f64,
        px: f64,
        py: f64,
        alloc: std.mem.Allocator,
    ) ?PaneId {
        var entries = std.ArrayListUnmanaged(LayoutEntry).empty;
        defer entries.deinit(alloc);
        self.layout(outer, divider_px, &entries, alloc);
        for (entries.items) |e| {
            if (e.rect.contains(px, py)) return e.id;
        }
        return null;
    }

    /// The leaf in `dir` direction from the focused leaf (geometric search),
    /// or null at an edge.
    pub fn neighbor(
        self: *const PaneTree,
        dir: enum { left, right, up, down },
        outer: Rect,
        divider_px: f64,
        alloc: std.mem.Allocator,
    ) ?PaneId {
        var entries = std.ArrayListUnmanaged(LayoutEntry).empty;
        defer entries.deinit(alloc);
        self.layout(outer, divider_px, &entries, alloc);

        var focused_rect: ?Rect = null;
        for (entries.items) |e| {
            if (e.id == self.focused) {
                focused_rect = e.rect;
                break;
            }
        }
        const fr = focused_rect orelse return null;
        const fr_cx = fr.centerX();
        const fr_cy = fr.centerY();

        var best_id: ?PaneId = null;
        var best_dist: f64 = std.math.inf(f64);

        for (entries.items) |e| {
            if (e.id == self.focused) continue;
            const r = e.rect;
            const qualifies = switch (dir) {
                .left => r.x + r.w <= fr.x + divider_px,
                .right => r.x >= fr.x + fr.w - divider_px,
                .up => r.y + r.h <= fr.y + divider_px,
                .down => r.y >= fr.y + fr.h - divider_px,
            };
            if (!qualifies) continue;
            const dx = r.centerX() - fr_cx;
            const dy = r.centerY() - fr_cy;
            const dist = dx * dx + dy * dy;
            if (dist < best_dist) {
                best_dist = dist;
                best_id = e.id;
            }
        }
        return best_id;
    }

    /// Number of leaf nodes in the tree.
    pub fn leafCount(self: *const PaneTree) usize {
        return countLeaves(self.root);
    }
};

/// Result of a divider hit-test: the split node that owns the divider, the
/// index of the child *before* the divider (child i+1 is on the other side),
/// and the divider's center position in device pixels along the split axis.
pub const DividerHit = struct {
    split_node: *PaneNode,
    child_index: usize, // divider is between children[child_index] and [child_index+1]
    axis_center: f64, // px position of the divider center (x for horizontal, y for vertical)
};

/// Find the divider closest to `px, py` within `slop_px` device pixels.
/// Returns null when the point is not near any divider.
/// No allocation — the tree is traversed directly.
pub fn findDividerAt(
    tree: *const PaneTree,
    outer: Rect,
    divider_px: f64,
    px: f64,
    py: f64,
    slop_px: f64,
) ?DividerHit {
    return findDividerInNode(tree.root, outer, divider_px, px, py, slop_px);
}

fn findDividerInNode(
    node: *PaneNode,
    rect: Rect,
    divider_px: f64,
    px: f64,
    py: f64,
    slop_px: f64,
) ?DividerHit {
    switch (node.*) {
        .leaf => return null,
        .split => |*sp| {
            const n = sp.children.items.len;
            const total_gutter = divider_px * @as(f64, @floatFromInt(n - 1));
            const available = switch (sp.dir) {
                .horizontal => rect.w - total_gutter,
                .vertical => rect.h - total_gutter,
            };
            var offset: f64 = 0.0;
            for (sp.children.items, 0..) |child, i| {
                const child_size = sp.ratios.items[i] * available;
                const child_rect: Rect = switch (sp.dir) {
                    .horizontal => .{ .x = rect.x + offset, .y = rect.y, .w = child_size, .h = rect.h },
                    .vertical => .{ .x = rect.x, .y = rect.y + offset, .w = rect.w, .h = child_size },
                };
                offset += child_size;

                // Check the gutter *after* this child (not after the last child).
                if (i + 1 < n) {
                    const gutter_start = switch (sp.dir) {
                        .horizontal => rect.x + offset,
                        .vertical => rect.y + offset,
                    };
                    const gutter_center = gutter_start + divider_px * 0.5;
                    const hit_coord = switch (sp.dir) {
                        .horizontal => px,
                        .vertical => py,
                    };
                    // For a horizontal split, the click must also be within the
                    // vertical span of this split (and vice versa).
                    const in_span = switch (sp.dir) {
                        .horizontal => py >= rect.y and py < rect.y + rect.h,
                        .vertical => px >= rect.x and px < rect.x + rect.w,
                    };
                    if (in_span and @abs(hit_coord - gutter_center) <= divider_px * 0.5 + slop_px) {
                        return .{
                            .split_node = node,
                            .child_index = i,
                            .axis_center = gutter_center,
                        };
                    }
                    offset += divider_px;

                    // Recurse into child's subtree.
                    const child_result = findDividerInNode(child, child_rect, divider_px, px, py, slop_px);
                    if (child_result) |hit| return hit;
                } else {
                    // Last child: only recurse.
                    const child_result = findDividerInNode(child, child_rect, divider_px, px, py, slop_px);
                    if (child_result) |hit| return hit;
                }
            }
            return null;
        },
    }
}

/// Move `delta` ratio units between `ratios[divider_index]` and
/// `ratios[divider_index+1]`. Positive delta grows index i, shrinks i+1.
/// Each ratio is clamped to `min_ratio`; the pair-sum invariant is preserved.
pub fn adjustRatio(sp: *Split, divider_index: usize, delta: f64, min_ratio: f64) void {
    const i = divider_index;
    const j = divider_index + 1;
    std.debug.assert(j < sp.ratios.items.len);
    const total = sp.ratios.items[i] + sp.ratios.items[j];
    const new_i = @max(min_ratio, @min(total - min_ratio, sp.ratios.items[i] + delta));
    sp.ratios.items[i] = new_i;
    sp.ratios.items[j] = total - new_i;
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Recursively free a node and all its descendants.
fn freeNode(alloc: std.mem.Allocator, node: *PaneNode) void {
    switch (node.*) {
        .leaf => {},
        .split => |*sp| {
            for (sp.children.items) |child| freeNode(alloc, child);
            sp.children.deinit(alloc);
            sp.ratios.deinit(alloc);
        },
    }
    alloc.destroy(node);
}

/// Walk result: the found node and its immediate parent split node (null for root).
const FindResult = struct {
    node: *PaneNode,
    parent: ?*PaneNode,
};

/// Find the leaf `id` returning the node and its direct parent split (null for root).
/// Returns null if `id` is not present.
fn findIn(node: *PaneNode, id: PaneId, parent: ?*PaneNode) ?FindResult {
    switch (node.*) {
        .leaf => |leaf_id| {
            if (leaf_id == id) return .{ .node = node, .parent = parent };
            return null;
        },
        .split => |*sp| {
            for (sp.children.items) |child| {
                if (findIn(child, id, node)) |r| return r;
            }
            return null;
        },
    }
}

/// Return the index of `child` in `sp.children`. Panics if not found.
fn findChildIndex(sp: *const Split, child: *PaneNode) usize {
    for (sp.children.items, 0..) |c, i| {
        if (c == child) return i;
    }
    @panic("child not found in split");
}

/// Insert `new_node` into `sp` at `idx` and re-equalize all ratios.
fn insertSibling(
    sp: *Split,
    alloc: std.mem.Allocator,
    new_node: *PaneNode,
    idx: usize,
) !void {
    try sp.children.insert(alloc, idx, new_node);
    try sp.ratios.insert(alloc, idx, 0.0); // placeholder; renormalize below
    const n = sp.children.items.len;
    const equal_share = 1.0 / @as(f64, @floatFromInt(n));
    for (sp.ratios.items) |*r| r.* = equal_share;
}

/// Replace `old` with `new` in `sp.children`. Panics if `old` not found.
fn replaceInSplit(sp: *Split, old: *PaneNode, new: *PaneNode) void {
    for (sp.children.items) |*c| {
        if (c.* == old) {
            c.* = new;
            return;
        }
    }
    @panic("old not found in split");
}

/// After a split has been reduced to one child, collapse it:
/// replace `parent_node` with `surviving` in the grandparent, or make
/// `surviving` the new root. Frees `parent_node`'s split shell.
fn collapseParent(tree: *PaneTree, parent_node: *PaneNode, surviving: *PaneNode) void {
    const sp = &parent_node.split;
    if (parent_node == tree.root) {
        // The collapsing split IS the root. Make surviving the new root by
        // overwriting root.* in place (keeps the root pointer stable) then
        // freeing the surviving shell. But surviving may itself be a split or
        // a leaf — we copy its contents into root and free the surviving pointer.
        const saved = surviving.*;
        sp.children.deinit(tree.alloc);
        sp.ratios.deinit(tree.alloc);
        tree.root.* = saved;
        tree.alloc.destroy(surviving);
    } else {
        // Find the grandparent split that holds parent_node.
        const gp = findParentOf(tree.root, parent_node) orelse
            @panic("parent_node not found in tree");
        replaceInSplit(&gp.split, parent_node, surviving);
        sp.children.deinit(tree.alloc);
        sp.ratios.deinit(tree.alloc);
        tree.alloc.destroy(parent_node);
    }
}

/// Find the split node that directly contains `target` as a child.
/// Returns null if `target` is the root (no parent) or if not found.
fn findParentOf(root: *PaneNode, target: *PaneNode) ?*PaneNode {
    return findParentIn(root, target);
}

fn findParentIn(node: *PaneNode, target: *PaneNode) ?*PaneNode {
    switch (node.*) {
        .leaf => return null,
        .split => |*sp| {
            for (sp.children.items) |child| {
                if (child == target) return node;
                if (findParentIn(child, target)) |p| return p;
            }
            return null;
        },
    }
}

/// Recurse into the tree, computing and emitting one LayoutEntry per leaf.
fn layoutNode(
    node: *const PaneNode,
    rect: Rect,
    divider_px: f64,
    out: *std.ArrayListUnmanaged(PaneTree.LayoutEntry),
    alloc: std.mem.Allocator,
) void {
    switch (node.*) {
        .leaf => |id| {
            out.append(alloc, .{ .id = id, .rect = rect }) catch return;
        },
        .split => |*sp| {
            const n = sp.children.items.len;
            const total_gutter = divider_px * @as(f64, @floatFromInt(n - 1));
            const available = switch (sp.dir) {
                .horizontal => rect.w - total_gutter,
                .vertical => rect.h - total_gutter,
            };
            var offset: f64 = 0.0;
            for (sp.children.items, 0..) |child, i| {
                const child_size = sp.ratios.items[i] * available;
                const child_rect: Rect = switch (sp.dir) {
                    .horizontal => .{ .x = rect.x + offset, .y = rect.y, .w = child_size, .h = rect.h },
                    .vertical => .{ .x = rect.x, .y = rect.y + offset, .w = rect.w, .h = child_size },
                };
                layoutNode(child, child_rect, divider_px, out, alloc);
                offset += child_size + divider_px;
            }
        },
    }
}

fn countLeaves(node: *const PaneNode) usize {
    return switch (node.*) {
        .leaf => 1,
        .split => |*sp| blk: {
            var total: usize = 0;
            for (sp.children.items) |child| total += countLeaves(child);
            break :blk total;
        },
    };
}

/// Return the PaneId of the first (leftmost/topmost) leaf under `node`.
fn firstLeafId(node: *const PaneNode) PaneId {
    return switch (node.*) {
        .leaf => |id| id,
        .split => |*sp| firstLeafId(sp.children.items[0]),
    };
}

/// Normalize `ratios` in place so they sum to 1.0.
fn renormalize(ratios: []f64) void {
    var sum: f64 = 0.0;
    for (ratios) |r| sum += r;
    if (sum == 0.0) {
        const eq = 1.0 / @as(f64, @floatFromInt(ratios.len));
        for (ratios) |*r| r.* = eq;
        return;
    }
    for (ratios) |*r| r.* *= 1.0 / sum;
}

/// Depth of the tree (a single leaf = 1; root split + leaf children = 2).
fn treeDepth(node: *const PaneNode) usize {
    return switch (node.*) {
        .leaf => 1,
        .split => |*sp| blk: {
            var max_child: usize = 0;
            for (sp.children.items) |child| {
                const d = treeDepth(child);
                if (d > max_child) max_child = d;
            }
            break :blk 1 + max_child;
        },
    };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

const testing = std.testing;

fn checkRatiosSumTree(node: *const PaneNode) !void {
    switch (node.*) {
        .leaf => {},
        .split => |*sp| {
            var s: f64 = 0.0;
            for (sp.ratios.items) |r| s += r;
            try testing.expectApproxEqAbs(1.0, s, 1e-9);
            for (sp.children.items) |child| try checkRatiosSumTree(child);
        },
    }
}

test "initSingle: single leaf, focused on it" {
    var tree = try PaneTree.initSingle(testing.allocator, 1);
    defer tree.deinit();
    try testing.expectEqual(@as(PaneId, 1), tree.focused);
    try testing.expectEqual(@as(usize, 1), tree.leafCount());
    try testing.expect(tree.root.* == .leaf);
    try testing.expectEqual(@as(PaneId, 1), tree.root.leaf);
}

test "split then close returns to single leaf" {
    var tree = try PaneTree.initSingle(testing.allocator, 1);
    defer tree.deinit();

    try tree.split(.horizontal, 2);
    try testing.expectEqual(@as(usize, 2), tree.leafCount());
    try testing.expectEqual(@as(PaneId, 2), tree.focused);

    const next = tree.closeLeaf(2);
    try testing.expectEqual(@as(?PaneId, 1), next);
    try testing.expectEqual(@as(PaneId, 1), tree.focused);
    try testing.expectEqual(@as(usize, 1), tree.leafCount());
    try testing.expect(tree.root.* == .leaf);
    try testing.expectEqual(@as(PaneId, 1), tree.root.leaf);
}

test "splitting same direction stays flat (sibling insert, no nesting)" {
    var tree = try PaneTree.initSingle(testing.allocator, 1);
    defer tree.deinit();

    // Focus=1; split horizontal -> [1, 2], focused=2.
    try tree.split(.horizontal, 2);
    // Focus=2; split horizontal again -> [1, 2, 3] flat, focused=3.
    try tree.split(.horizontal, 3);

    // Depth 2: root split + leaf children (no nesting).
    try testing.expectEqual(@as(usize, 2), treeDepth(tree.root));
    try testing.expectEqual(@as(usize, 3), tree.leafCount());
    try testing.expect(tree.root.* == .split);
    try testing.expectEqual(@as(usize, 3), tree.root.split.children.items.len);
    try checkRatiosSumTree(tree.root);
}

test "splitting opposite direction nests" {
    var tree = try PaneTree.initSingle(testing.allocator, 1);
    defer tree.deinit();

    // Horizontal split: [1, 2].
    try tree.split(.horizontal, 2);
    // Focused=2, vertical split: nests 2 -> [2, 3].
    try tree.split(.vertical, 3);

    // Depth 3: root split -> inner split -> leaves.
    try testing.expectEqual(@as(usize, 3), treeDepth(tree.root));
    try testing.expectEqual(@as(usize, 3), tree.leafCount());
    try checkRatiosSumTree(tree.root);
}

test "closeLeaf collapses single-child split" {
    // [1, 2] horizontal. Close 2 -> root becomes leaf 1.
    var tree = try PaneTree.initSingle(testing.allocator, 1);
    defer tree.deinit();

    try tree.split(.horizontal, 2);
    _ = tree.closeLeaf(2);

    try testing.expect(tree.root.* == .leaf);
    try testing.expectEqual(@as(PaneId, 1), tree.root.leaf);
    try testing.expectEqual(@as(usize, 1), tree.leafCount());
}

test "closeLeaf returns valid next-focus, never removed; null when empty" {
    // [1, 2, 3] flat horizontal.
    var tree = try PaneTree.initSingle(testing.allocator, 1);
    defer tree.deinit();

    try tree.split(.horizontal, 2);
    try tree.split(.horizontal, 3);

    // Close focused (3); must get back a sibling.
    const next = tree.closeLeaf(3);
    try testing.expect(next != null);
    try testing.expect(next.? != 3);
    try testing.expect(tree.focused != 3);

    // Close again.
    const remaining1 = tree.closeLeaf(next.?);
    try testing.expect(remaining1 != null);
    try testing.expect(remaining1.? != next.?);

    // Tree is now empty.
    const empty = tree.closeLeaf(remaining1.?);
    try testing.expectEqual(@as(?PaneId, null), empty);
}

test "ratios always sum to ~1.0 after split/close sequences" {
    var tree = try PaneTree.initSingle(testing.allocator, 1);
    defer tree.deinit();

    try tree.split(.horizontal, 2);
    try checkRatiosSumTree(tree.root);

    try tree.split(.horizontal, 3);
    try checkRatiosSumTree(tree.root);

    _ = tree.closeLeaf(3);
    try checkRatiosSumTree(tree.root);

    try tree.split(.vertical, 4);
    try checkRatiosSumTree(tree.root);
}

test "layout: rects tile outer minus gutters, non-overlapping" {
    const LayoutCase = struct {
        name: [:0]const u8,
        ids: []const PaneId,
        dirs: []const SplitDir,
        outer: Rect,
        div: f64,
    };
    const cases = [_]LayoutCase{
        .{
            .name = "two horizontal",
            .ids = &[_]PaneId{ 1, 2 },
            .dirs = &[_]SplitDir{.horizontal},
            .outer = .{ .x = 0, .y = 0, .w = 200, .h = 100 },
            .div = 4,
        },
        .{
            .name = "two vertical",
            .ids = &[_]PaneId{ 1, 2 },
            .dirs = &[_]SplitDir{.vertical},
            .outer = .{ .x = 0, .y = 0, .w = 200, .h = 100 },
            .div = 4,
        },
        .{
            .name = "three horizontal flat",
            .ids = &[_]PaneId{ 1, 2, 3 },
            .dirs = &[_]SplitDir{ .horizontal, .horizontal },
            .outer = .{ .x = 10, .y = 20, .w = 300, .h = 150 },
            .div = 2,
        },
    };

    inline for (cases) |c| {
        var tree = try PaneTree.initSingle(testing.allocator, c.ids[0]);
        defer tree.deinit();
        for (c.ids[1..], 0..) |id, i| {
            try tree.split(c.dirs[@min(i, c.dirs.len - 1)], id);
        }

        var out = std.ArrayListUnmanaged(PaneTree.LayoutEntry).empty;
        defer out.deinit(testing.allocator);
        tree.layout(c.outer, c.div, &out, testing.allocator);

        try testing.expectEqual(c.ids.len, out.items.len);

        // All rects within outer.
        for (out.items) |e| {
            try testing.expect(e.rect.x >= c.outer.x - 1e-9);
            try testing.expect(e.rect.y >= c.outer.y - 1e-9);
            try testing.expect(e.rect.x + e.rect.w <= c.outer.x + c.outer.w + 1e-9);
            try testing.expect(e.rect.y + e.rect.h <= c.outer.y + c.outer.h + 1e-9);
        }

        // No pairwise overlap.
        for (out.items, 0..) |a, ai| {
            for (out.items, 0..) |b, bi| {
                if (ai == bi) continue;
                const ox = a.rect.x < b.rect.x + b.rect.w - 1e-9 and
                    b.rect.x < a.rect.x + a.rect.w - 1e-9;
                const oy = a.rect.y < b.rect.y + b.rect.h - 1e-9 and
                    b.rect.y < a.rect.y + a.rect.h - 1e-9;
                if (ox and oy) {
                    std.debug.print("layout case '{s}': rects overlap: id={} and id={}\n", .{ c.name, a.id, b.id });
                    return error.TestUnexpectedResult;
                }
            }
        }

        // Total area = outer minus gutters.
        var total_area: f64 = 0.0;
        for (out.items) |e| total_area += e.rect.w * e.rect.h;
        const n: f64 = @floatFromInt(c.ids.len);
        const gutter_count = n - 1.0;
        const expected_area = switch (tree.root.split.dir) {
            .horizontal => (c.outer.w - gutter_count * c.div) * c.outer.h,
            .vertical => c.outer.w * (c.outer.h - gutter_count * c.div),
        };
        try testing.expectApproxEqAbs(expected_area, total_area, 1e-6);
    }
}

test "hitTest round-trips: center of each rect hits back to that leaf" {
    var tree = try PaneTree.initSingle(testing.allocator, 1);
    defer tree.deinit();
    try tree.split(.horizontal, 2);
    try tree.split(.vertical, 3);

    const outer: Rect = .{ .x = 0, .y = 0, .w = 400, .h = 300 };
    const div = 4.0;

    var out = std.ArrayListUnmanaged(PaneTree.LayoutEntry).empty;
    defer out.deinit(testing.allocator);
    tree.layout(outer, div, &out, testing.allocator);

    for (out.items) |e| {
        const cx = e.rect.centerX();
        const cy = e.rect.centerY();
        const hit = tree.hitTest(outer, div, cx, cy, testing.allocator);
        try testing.expectEqual(@as(?PaneId, e.id), hit);
    }

    // A point in the gutter returns null.
    // With divider_px=4 and two equal panes on a 400-wide outer:
    // Left pane: w=(400-4)/2=198, right edge at x=198.
    // Gutter: x=198..202. Point at x=199 is in the gutter.
    const gutter_hit = tree.hitTest(outer, div, 199.0, 150.0, testing.allocator);
    try testing.expectEqual(@as(?PaneId, null), gutter_hit);
}

test "neighbor: directional nav on 2x2-ish tree, edges return null" {
    // Layout: horizontal split [1, inner_vertical[2,3]]
    // left=1, right-top=2, right-bottom=3.
    var tree = try PaneTree.initSingle(testing.allocator, 1);
    defer tree.deinit();

    try tree.split(.horizontal, 2); // [1, 2], focused=2
    try tree.split(.vertical, 3); // [1, [2,3]], focused=3

    const outer: Rect = .{ .x = 0, .y = 0, .w = 400, .h = 300 };
    const div = 4.0;

    // Focus=3 (bottom-right). Up -> 2.
    tree.focused = 3;
    try testing.expectEqual(@as(?PaneId, 2), tree.neighbor(.up, outer, div, testing.allocator));

    // Down from 3 -> null (edge).
    try testing.expectEqual(@as(?PaneId, null), tree.neighbor(.down, outer, div, testing.allocator));

    // Left from 3 -> 1.
    try testing.expectEqual(@as(?PaneId, 1), tree.neighbor(.left, outer, div, testing.allocator));

    // Focus=1; right -> 2 or 3 (nearest in right column).
    tree.focused = 1;
    const right = tree.neighbor(.right, outer, div, testing.allocator);
    try testing.expect(right != null);
    try testing.expect(right.? == 2 or right.? == 3);

    // Left from 1 -> null (edge).
    try testing.expectEqual(@as(?PaneId, null), tree.neighbor(.left, outer, div, testing.allocator));
}

test "adjustRatio: keeps sum at 1.0, every ratio >= min_ratio" {
    var tree = try PaneTree.initSingle(testing.allocator, 1);
    defer tree.deinit();
    try tree.split(.horizontal, 2);
    try tree.split(.horizontal, 3);

    // 3-child horizontal split. Adjust ratio between children 0 and 1.
    const sp = &tree.root.split;

    var prng = std.Random.DefaultPrng.init(42);
    const rand = prng.random();
    const min_r = 0.05;

    var i: usize = 0;
    while (i < 100) : (i += 1) {
        const delta = (rand.float(f64) - 0.5) * 0.4; // [-0.2, 0.2]
        adjustRatio(sp, 0, delta, min_r);

        var sum: f64 = 0.0;
        for (sp.ratios.items) |r| sum += r;
        try testing.expectApproxEqAbs(1.0, sum, 1e-9);

        try testing.expect(sp.ratios.items[0] >= min_r - 1e-9);
        try testing.expect(sp.ratios.items[1] >= min_r - 1e-9);
    }
}

test "closeLeaf on non-root: collapse propagates correctly" {
    // [1, [2, 3]]. Close 2: inner split collapses to [1, 3].
    var tree = try PaneTree.initSingle(testing.allocator, 1);
    defer tree.deinit();

    try tree.split(.horizontal, 2); // [1, 2], focused=2
    try tree.split(.vertical, 3); // [1, [2,3]], focused=3

    tree.focused = 2;
    _ = tree.closeLeaf(2);

    // Tree should now be [1, 3] flat.
    try testing.expectEqual(@as(usize, 2), tree.leafCount());
    try testing.expect(tree.root.* == .split);
    try testing.expectEqual(@as(usize, 2), tree.root.split.children.items.len);
    try checkRatiosSumTree(tree.root);
}

/// Helper: derive (cols, rows) from a pane rect, matching the resize path.
fn deriveColsRows(rect: Rect, cell_w: f64, cell_h: f64) struct { cols: usize, rows: usize } {
    const cols = @max(@as(usize, @intFromFloat(rect.w / cell_w)), 1);
    const rows = @max(@as(usize, @intFromFloat(rect.h / cell_h)), 1);
    return .{ .cols = cols, .rows = rows };
}

test "resize derivation: 2-pane horizontal split yields correct per-pane cols" {
    // A 2-pane horizontal split: each pane gets half the width minus half the gutter.
    var tree = try PaneTree.initSingle(testing.allocator, 1);
    defer tree.deinit();
    try tree.split(.horizontal, 2);

    const cell_w: f64 = 8.0;
    const cell_h: f64 = 16.0;
    const div: f64 = 4.0;
    const inner: Rect = .{ .x = 0, .y = 0, .w = 400, .h = 200 };

    var out = std.ArrayListUnmanaged(PaneTree.LayoutEntry).empty;
    defer out.deinit(testing.allocator);
    tree.layout(inner, div, &out, testing.allocator);

    try testing.expectEqual(@as(usize, 2), out.items.len);
    for (out.items) |e| {
        const dr = deriveColsRows(e.rect, cell_w, cell_h);
        // Each pane is (400 - 4) / 2 = 198 px wide -> 198 / 8 = 24 cols.
        try testing.expectEqual(@as(usize, 24), dr.cols);
        // Full height: 200 / 16 = 12 rows.
        try testing.expectEqual(@as(usize, 12), dr.rows);
        try testing.expect(dr.cols >= 1);
        try testing.expect(dr.rows >= 1);
    }
}

test "resize derivation: 3-pane vertical split yields >= 1 rows each" {
    var tree = try PaneTree.initSingle(testing.allocator, 1);
    defer tree.deinit();
    try tree.split(.vertical, 2);
    try tree.split(.vertical, 3);

    const cell_w: f64 = 8.0;
    const cell_h: f64 = 16.0;
    const div: f64 = 4.0;
    const inner: Rect = .{ .x = 0, .y = 0, .w = 300, .h = 150 };

    var out = std.ArrayListUnmanaged(PaneTree.LayoutEntry).empty;
    defer out.deinit(testing.allocator);
    tree.layout(inner, div, &out, testing.allocator);

    try testing.expectEqual(@as(usize, 3), out.items.len);
    for (out.items) |e| {
        const dr = deriveColsRows(e.rect, cell_w, cell_h);
        try testing.expect(dr.cols >= 1);
        try testing.expect(dr.rows >= 1);
    }
}

test "adjustRatio: a known delta shifts child ratio by that amount (before clamping)" {
    var tree = try PaneTree.initSingle(testing.allocator, 1);
    defer tree.deinit();
    try tree.split(.horizontal, 2);
    const sp = &tree.root.split;

    // Start: both at 0.5.
    const before_i = sp.ratios.items[0];
    const before_j = sp.ratios.items[1];
    const delta = 0.1;
    adjustRatio(sp, 0, delta, 0.05);

    // After: i should be 0.5+0.1=0.6, j = 0.4 (no clamping since both > min).
    try testing.expectApproxEqAbs(before_i + delta, sp.ratios.items[0], 1e-9);
    try testing.expectApproxEqAbs(before_j - delta, sp.ratios.items[1], 1e-9);
    // Sum still 1.0.
    var sum: f64 = 0.0;
    for (sp.ratios.items) |r| sum += r;
    try testing.expectApproxEqAbs(1.0, sum, 1e-9);
}

test "adjustRatio: delta clamped at min_ratio floor" {
    var tree = try PaneTree.initSingle(testing.allocator, 1);
    defer tree.deinit();
    try tree.split(.horizontal, 2);
    const sp = &tree.root.split;

    const min_r = 0.2;
    // A huge positive delta: j cannot go below min_r.
    adjustRatio(sp, 0, 1.0, min_r);
    try testing.expect(sp.ratios.items[1] >= min_r - 1e-9);
    try testing.expect(sp.ratios.items[0] >= min_r - 1e-9);
    var sum: f64 = 0.0;
    for (sp.ratios.items) |r| sum += r;
    try testing.expectApproxEqAbs(1.0, sum, 1e-9);
}

test "findDividerAt: finds the divider between two horizontal panes" {
    var tree = try PaneTree.initSingle(testing.allocator, 1);
    defer tree.deinit();
    try tree.split(.horizontal, 2);

    const outer: Rect = .{ .x = 0, .y = 0, .w = 400, .h = 200 };
    const div: f64 = 8.0;
    // Divider center: at x = (400 - 8) / 2 + 8/2 = 196 + 4 = 200.
    const divider_center_x: f64 = 200.0;

    // Click exactly at center.
    const hit = findDividerAt(&tree, outer, div, divider_center_x, 100.0, 4.0);
    try testing.expect(hit != null);
    try testing.expectEqual(@as(usize, 0), hit.?.child_index);

    // Click well outside: no hit.
    const miss = findDividerAt(&tree, outer, div, 50.0, 100.0, 4.0);
    try testing.expect(miss == null);
}

test "leafCount == registry-count invariant after split and close" {
    // Simulate the invariant: registry.count() should equal tree.leafCount()
    // after every split and close operation.
    // We fake the registry as a simple counter to avoid spawning PTYs.
    var tree = try PaneTree.initSingle(testing.allocator, 1);
    defer tree.deinit();
    var reg_count: usize = 1;

    try tree.split(.horizontal, 2);
    reg_count += 1;
    try testing.expectEqual(reg_count, tree.leafCount());

    try tree.split(.vertical, 3);
    reg_count += 1;
    try testing.expectEqual(reg_count, tree.leafCount());

    _ = tree.closeLeaf(3);
    reg_count -= 1;
    try testing.expectEqual(reg_count, tree.leafCount());

    _ = tree.closeLeaf(2);
    reg_count -= 1;
    try testing.expectEqual(reg_count, tree.leafCount());

    // Last pane close returns null.
    const last = tree.closeLeaf(1);
    try testing.expectEqual(@as(?PaneId, null), last);
    // tree is now empty; don't count leaves.
}
