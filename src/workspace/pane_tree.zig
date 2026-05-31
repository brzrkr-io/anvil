const std = @import("std");

/// Split axis. `.x` divides width (panes side by side); `.y` divides height
/// (panes stacked top/bottom).
pub const Axis = enum { x, y };

pub const Dir = enum { left, right, up, down };

pub const Rect = struct { x: f32, y: f32, w: f32, h: f32 };

pub const PaneRect = struct { id: usize, rect: Rect };

const Node = union(enum) {
    leaf: usize, // opaque session id
    split: *Split,
};

const Split = struct {
    axis: Axis,
    ratio: f32, // first child's fraction of the divided extent
    a: Node,
    b: Node,
};

/// Public mirror of Node used for serialization; owns its own memory via the
/// provided allocator. Free with `freeExport`.
pub const NodeExport = union(enum) {
    leaf: usize,
    split: *SplitExport,
};

pub const SplitExport = struct {
    axis: Axis,
    ratio: f32,
    a: *NodeExport,
    b: *NodeExport,
};

/// A binary tree of split panes. Leaves carry opaque session ids; the tree
/// owns only structure and layout geometry. Focus lives in the caller.
pub const PaneTree = struct {
    alloc: std.mem.Allocator,
    root: Node,

    pub fn init(alloc: std.mem.Allocator, first_id: usize) PaneTree {
        return .{ .alloc = alloc, .root = .{ .leaf = first_id } };
    }

    pub fn deinit(self: *PaneTree) void {
        freeNode(self.alloc, self.root);
    }

    fn freeNode(alloc: std.mem.Allocator, node: Node) void {
        switch (node) {
            .leaf => {},
            .split => |sp| {
                freeNode(alloc, sp.a);
                freeNode(alloc, sp.b);
                alloc.destroy(sp);
            },
        }
    }

    pub fn count(self: *const PaneTree) usize {
        return countNode(self.root);
    }

    fn countNode(node: Node) usize {
        return switch (node) {
            .leaf => 1,
            .split => |sp| countNode(sp.a) + countNode(sp.b),
        };
    }

    /// Split the leaf carrying `target` into two along `axis`; `new_id` becomes
    /// the second pane. No-op if `target` is not present.
    pub fn split(self: *PaneTree, target: usize, axis: Axis, new_id: usize) !void {
        return self.splitWithRatio(target, axis, new_id, 0.5);
    }

    /// Like split but uses the given ratio for the first child's fraction.
    pub fn splitWithRatio(self: *PaneTree, target: usize, axis: Axis, new_id: usize, ratio: f32) !void {
        const slot = findLeaf(&self.root, target) orelse return;
        const sp = try self.alloc.create(Split);
        sp.* = .{ .axis = axis, .ratio = ratio, .a = slot.*, .b = .{ .leaf = new_id } };
        slot.* = .{ .split = sp };
    }

    /// Split with the new pane as the FIRST child (top/left); the existing pane
    /// becomes the second (bottom/right). `ratio` is the new pane's fraction.
    pub fn splitNewFirst(self: *PaneTree, target: usize, axis: Axis, new_id: usize, ratio: f32) !void {
        const slot = findLeaf(&self.root, target) orelse return;
        const sp = try self.alloc.create(Split);
        sp.* = .{ .axis = axis, .ratio = ratio, .a = .{ .leaf = new_id }, .b = slot.* };
        slot.* = .{ .split = sp };
    }

    /// Remove the leaf carrying `id`, collapsing its parent into the sibling.
    /// No-op if `id` is absent or is the only pane.
    pub fn close(self: *PaneTree, id: usize) void {
        _ = self.removeFrom(&self.root, id);
    }

    fn removeFrom(self: *PaneTree, node: *Node, id: usize) bool {
        switch (node.*) {
            .leaf => return false,
            .split => |sp| {
                if (leafId(sp.a) == id) {
                    const sib = sp.b;
                    self.alloc.destroy(sp);
                    node.* = sib;
                    return true;
                }
                if (leafId(sp.b) == id) {
                    const sib = sp.a;
                    self.alloc.destroy(sp);
                    node.* = sib;
                    return true;
                }
                return self.removeFrom(&sp.a, id) or self.removeFrom(&sp.b, id);
            },
        }
    }

    /// Grow the pane carrying `target` toward `dir` by `step` (fraction of the
    /// relevant extent). Moves the nearest ancestor divider whose axis matches
    /// the direction; no-op if the pane is already at that edge of the window.
    pub fn resize(self: *PaneTree, target: usize, dir: Dir, step: f32) void {
        const axis: Axis = if (dir == .left or dir == .right) .x else .y;
        const grow_a = (dir == .right or dir == .down); // target in `a` grows by +step
        var node: *Node = &self.root;
        var chosen: ?*Split = null;
        while (true) {
            switch (node.*) {
                .leaf => break,
                .split => |sp| {
                    const in_a = subtreeHas(sp.a, target);
                    if (sp.axis == axis) {
                        // Pick the split where moving the divider grows the target:
                        // target in `a` needs +dir; target in `b` needs -dir.
                        if (grow_a and in_a) chosen = sp;
                        if (!grow_a and !in_a) chosen = sp;
                    }
                    node = if (in_a) &sp.a else &sp.b;
                },
            }
        }
        if (chosen) |sp| {
            const d: f32 = if (grow_a) step else -step;
            sp.ratio = std.math.clamp(sp.ratio + d, 0.1, 0.9);
        }
    }

    /// Reset every split to an even 50/50, recursively.
    pub fn balance(self: *PaneTree) void {
        balanceNode(self.root);
    }

    fn balanceNode(node: Node) void {
        switch (node) {
            .leaf => {},
            .split => |sp| {
                sp.ratio = 0.5;
                balanceNode(sp.a);
                balanceNode(sp.b);
            },
        }
    }

    /// Lay panes out within `rect`, leaving `divider` device pixels between
    /// siblings. Fills `out` (size >= count()) and returns the pane count.
    pub fn layout(self: *const PaneTree, rect: Rect, divider: f32, out: []PaneRect) usize {
        var n: usize = 0;
        layoutNode(self.root, rect, divider, out, &n);
        return n;
    }

    fn layoutNode(node: Node, rect: Rect, divider: f32, out: []PaneRect, n: *usize) void {
        switch (node) {
            .leaf => |id| {
                out[n.*] = .{ .id = id, .rect = rect };
                n.* += 1;
            },
            .split => |sp| {
                const ra, const rb = splitRect(rect, sp.axis, sp.ratio, divider);
                layoutNode(sp.a, ra, divider, out, n);
                layoutNode(sp.b, rb, divider, out, n);
            },
        }
    }

    /// Emit a rect for each split's divider gap. Fills `out` (size >=
    /// count()-1) and returns the divider count.
    pub fn dividers(self: *const PaneTree, rect: Rect, thickness: f32, out: []Rect) usize {
        var n: usize = 0;
        dividerNode(self.root, rect, thickness, out, &n);
        return n;
    }

    fn dividerNode(node: Node, rect: Rect, thickness: f32, out: []Rect, n: *usize) void {
        switch (node) {
            .leaf => {},
            .split => |sp| {
                const ra, const rb = splitRect(rect, sp.axis, sp.ratio, thickness);
                out[n.*] = switch (sp.axis) {
                    .x => .{ .x = ra.x + ra.w, .y = rect.y, .w = thickness, .h = rect.h },
                    .y => .{ .x = rect.x, .y = ra.y + ra.h, .w = rect.w, .h = thickness },
                };
                n.* += 1;
                dividerNode(sp.a, ra, thickness, out, n);
                dividerNode(sp.b, rb, thickness, out, n);
            },
        }
    }

    /// The leaf nearest to `from` in `dir`, given a layout within `rect`.
    /// Returns null if there is no pane in that direction.
    pub fn neighbor(self: *const PaneTree, rect: Rect, from: usize, dir: Dir, buf: []PaneRect) ?usize {
        const n = self.layout(rect, 0, buf);
        var src: ?Rect = null;
        for (buf[0..n]) |p| {
            if (p.id == from) src = p.rect;
        }
        const f = src orelse return null;
        var best: ?usize = null;
        var best_d: f32 = std.math.floatMax(f32);
        for (buf[0..n]) |p| {
            if (p.id == from) continue;
            if (!inDir(f, p.rect, dir)) continue;
            const d = dirDist(f, p.rect, dir);
            if (d < best_d) {
                best_d = d;
                best = p.id;
            }
        }
        return best;
    }

    fn firstLeaf(node: Node) usize {
        return switch (node) {
            .leaf => |id| id,
            .split => |sp| firstLeaf(sp.a),
        };
    }

    /// Any surviving leaf id — useful for re-homing focus after a close.
    pub fn anyLeaf(self: *const PaneTree) usize {
        return firstLeaf(self.root);
    }

    /// Export the tree structure for serialization. Caller owns the returned
    /// memory and must call `freeExport` when done.
    pub fn exportRoot(self: *const PaneTree, alloc: std.mem.Allocator) !NodeExport {
        return exportNode(alloc, self.root);
    }

    fn exportNode(alloc: std.mem.Allocator, node: Node) !NodeExport {
        switch (node) {
            .leaf => |id| return .{ .leaf = id },
            .split => |sp| {
                const ex = try alloc.create(SplitExport);
                const a_ptr = try alloc.create(NodeExport);
                a_ptr.* = try exportNode(alloc, sp.a);
                const b_ptr = try alloc.create(NodeExport);
                b_ptr.* = try exportNode(alloc, sp.b);
                ex.* = .{ .axis = sp.axis, .ratio = sp.ratio, .a = a_ptr, .b = b_ptr };
                return .{ .split = ex };
            },
        }
    }

    pub fn freeExport(alloc: std.mem.Allocator, node: NodeExport) void {
        switch (node) {
            .leaf => {},
            .split => |sp| {
                freeExport(alloc, sp.a.*);
                freeExport(alloc, sp.b.*);
                alloc.destroy(sp.a);
                alloc.destroy(sp.b);
                alloc.destroy(sp);
            },
        }
    }

    /// Collect every leaf id into `out` (size >= count()); returns the count.
    pub fn leaves(self: *const PaneTree, out: []usize) usize {
        var n: usize = 0;
        leafIds(self.root, out, &n);
        return n;
    }

    fn leafIds(node: Node, out: []usize, n: *usize) void {
        switch (node) {
            .leaf => |id| {
                out[n.*] = id;
                n.* += 1;
            },
            .split => |sp| {
                leafIds(sp.a, out, n);
                leafIds(sp.b, out, n);
            },
        }
    }
};

fn leafId(node: Node) ?usize {
    return switch (node) {
        .leaf => |id| id,
        .split => null,
    };
}

fn subtreeHas(node: Node, id: usize) bool {
    return switch (node) {
        .leaf => |lid| lid == id,
        .split => |sp| subtreeHas(sp.a, id) or subtreeHas(sp.b, id),
    };
}

fn findLeaf(node: *Node, id: usize) ?*Node {
    switch (node.*) {
        .leaf => |lid| return if (lid == id) node else null,
        .split => |sp| return findLeaf(&sp.a, id) orelse findLeaf(&sp.b, id),
    }
}

fn splitRect(r: Rect, axis: Axis, ratio: f32, divider: f32) struct { Rect, Rect } {
    return switch (axis) {
        .x => blk: {
            const usable = r.w - divider;
            const aw = usable * ratio;
            break :blk .{
                .{ .x = r.x, .y = r.y, .w = aw, .h = r.h },
                .{ .x = r.x + aw + divider, .y = r.y, .w = usable - aw, .h = r.h },
            };
        },
        .y => blk: {
            const usable = r.h - divider;
            const ah = usable * ratio;
            break :blk .{
                .{ .x = r.x, .y = r.y, .w = r.w, .h = ah },
                .{ .x = r.x, .y = r.y + ah + divider, .w = r.w, .h = usable - ah },
            };
        },
    };
}

fn center(r: Rect) struct { f32, f32 } {
    return .{ r.x + r.w / 2, r.y + r.h / 2 };
}

fn inDir(from: Rect, to: Rect, dir: Dir) bool {
    const fc = center(from);
    const tc = center(to);
    return switch (dir) {
        .left => tc[0] < fc[0] and overlap(from.y, from.h, to.y, to.h),
        .right => tc[0] > fc[0] and overlap(from.y, from.h, to.y, to.h),
        .up => tc[1] < fc[1] and overlap(from.x, from.w, to.x, to.w),
        .down => tc[1] > fc[1] and overlap(from.x, from.w, to.x, to.w),
    };
}

fn overlap(a0: f32, alen: f32, b0: f32, blen: f32) bool {
    return a0 < b0 + blen and b0 < a0 + alen;
}

fn dirDist(from: Rect, to: Rect, dir: Dir) f32 {
    const fc = center(from);
    const tc = center(to);
    return switch (dir) {
        .left, .right => @abs(tc[0] - fc[0]),
        .up, .down => @abs(tc[1] - fc[1]),
    };
}

const t = std.testing;

test "single pane fills the rect" {
    var tree = PaneTree.init(t.allocator, 0);
    defer tree.deinit();
    var buf: [8]PaneRect = undefined;
    const n = tree.layout(.{ .x = 0, .y = 0, .w = 100, .h = 50 }, 0, &buf);
    try t.expectEqual(@as(usize, 1), n);
    try t.expectEqual(@as(f32, 100), buf[0].rect.w);
}

test "x split halves the width with a divider gap" {
    var tree = PaneTree.init(t.allocator, 0);
    defer tree.deinit();
    try tree.split(0, .x, 1);
    var buf: [8]PaneRect = undefined;
    const n = tree.layout(.{ .x = 0, .y = 0, .w = 100, .h = 40 }, 2, &buf);
    try t.expectEqual(@as(usize, 2), n);
    try t.expectEqual(@as(f32, 49), buf[0].rect.w); // (100-2)*0.5
    try t.expectEqual(@as(f32, 51), buf[1].rect.x); // 49 + 2
    try t.expectEqual(@as(f32, 49), buf[1].rect.w);
}

test "y split halves the height" {
    var tree = PaneTree.init(t.allocator, 0);
    defer tree.deinit();
    try tree.split(0, .y, 1);
    var buf: [8]PaneRect = undefined;
    const n = tree.layout(.{ .x = 0, .y = 0, .w = 80, .h = 100 }, 0, &buf);
    try t.expectEqual(@as(usize, 2), n);
    try t.expectEqual(@as(f32, 50), buf[0].rect.h);
    try t.expectEqual(@as(f32, 50), buf[1].rect.y);
}

test "splitNewFirst y stacks the new pane on top, full width" {
    // Opening a file from the explorer puts the editor ABOVE the terminal:
    // the new pane must be the first (top) child, full width, at the ratio.
    var tree = PaneTree.init(t.allocator, 0); // existing terminal = id 0
    defer tree.deinit();
    try tree.splitNewFirst(0, .y, 1, 0.7); // editor = id 1 on top
    var buf: [8]PaneRect = undefined;
    const n = tree.layout(.{ .x = 0, .y = 0, .w = 80, .h = 100 }, 0, &buf);
    try t.expectEqual(@as(usize, 2), n);
    // buf[0] is the new (editor) pane: top, full width, 70% tall.
    try t.expectEqual(@as(usize, 1), buf[0].id);
    try t.expectEqual(@as(f32, 0), buf[0].rect.y);
    try t.expectEqual(@as(f32, 80), buf[0].rect.w);
    try t.expectEqual(@as(f32, 70), buf[0].rect.h);
    // buf[1] is the old (terminal) pane: directly below, full width.
    try t.expectEqual(@as(usize, 0), buf[1].id);
    try t.expectEqual(@as(f32, 70), buf[1].rect.y);
    try t.expectEqual(@as(f32, 80), buf[1].rect.w);
    try t.expectEqual(@as(f32, 30), buf[1].rect.h);
}

test "close collapses the parent into the sibling" {
    var tree = PaneTree.init(t.allocator, 0);
    defer tree.deinit();
    try tree.split(0, .x, 1); // panes 0 | 1
    try t.expectEqual(@as(usize, 2), tree.count());
    tree.close(1);
    try t.expectEqual(@as(usize, 1), tree.count());
    var buf: [8]PaneRect = undefined;
    const n = tree.layout(.{ .x = 0, .y = 0, .w = 100, .h = 50 }, 0, &buf);
    try t.expectEqual(@as(usize, 1), n);
    try t.expectEqual(@as(usize, 0), buf[0].id); // sibling 0 now fills
}

test "close is a no-op on the last pane" {
    var tree = PaneTree.init(t.allocator, 0);
    defer tree.deinit();
    tree.close(0);
    try t.expectEqual(@as(usize, 1), tree.count());
}

test "nested split then close collapses correctly" {
    var tree = PaneTree.init(t.allocator, 0);
    defer tree.deinit();
    try tree.split(0, .x, 1); // 0 | 1
    try tree.split(1, .y, 2); // 0 | (1 / 2)
    try t.expectEqual(@as(usize, 3), tree.count());
    tree.close(2); // right side collapses back to leaf 1
    try t.expectEqual(@as(usize, 2), tree.count());
    var buf: [8]PaneRect = undefined;
    const n = tree.layout(.{ .x = 0, .y = 0, .w = 100, .h = 100 }, 0, &buf);
    try t.expectEqual(@as(usize, 2), n);
}

test "dividers emit one gap rect per split" {
    var tree = PaneTree.init(t.allocator, 0);
    defer tree.deinit();
    try tree.split(0, .x, 1); // one vertical divider
    var buf: [8]Rect = undefined;
    const n = tree.dividers(.{ .x = 0, .y = 0, .w = 100, .h = 40 }, 2, &buf);
    try t.expectEqual(@as(usize, 1), n);
    try t.expectEqual(@as(f32, 49), buf[0].x); // gap starts after pane a
    try t.expectEqual(@as(f32, 2), buf[0].w);
    try t.expectEqual(@as(f32, 40), buf[0].h);
}

test "resize grows the focused pane toward the divider" {
    var tree = PaneTree.init(t.allocator, 0);
    defer tree.deinit();
    try tree.split(0, .x, 1); // 0 | 1, ratio 0.5
    tree.resize(0, .right, 0.1); // grow left pane rightward
    var buf: [8]PaneRect = undefined;
    const n = tree.layout(.{ .x = 0, .y = 0, .w = 100, .h = 40 }, 0, &buf);
    try t.expectEqual(@as(usize, 2), n);
    try t.expectApproxEqAbs(@as(f32, 60), buf[0].rect.w, 0.001); // 0.6 * 100
    // Pane 1 grows leftward (ratio shrinks back toward 0.5).
    tree.resize(1, .left, 0.1);
    _ = tree.layout(.{ .x = 0, .y = 0, .w = 100, .h = 40 }, 0, &buf);
    try t.expectApproxEqAbs(@as(f32, 50), buf[0].rect.w, 0.001);
}

test "resize is a no-op at the window edge" {
    var tree = PaneTree.init(t.allocator, 0);
    defer tree.deinit();
    try tree.split(0, .x, 1); // only an x-split exists
    tree.resize(0, .up, 0.1); // no y-split to move
    var buf: [8]PaneRect = undefined;
    _ = tree.layout(.{ .x = 0, .y = 0, .w = 100, .h = 40 }, 0, &buf);
    try t.expectApproxEqAbs(@as(f32, 50), buf[0].rect.w, 0.001);
}

test "balance resets nested splits to 50/50" {
    var tree = PaneTree.init(t.allocator, 0);
    defer tree.deinit();
    try tree.split(0, .x, 1);
    tree.resize(0, .right, 0.3); // skew it
    tree.balance();
    var buf: [8]PaneRect = undefined;
    _ = tree.layout(.{ .x = 0, .y = 0, .w = 100, .h = 40 }, 0, &buf);
    try t.expectApproxEqAbs(@as(f32, 50), buf[0].rect.w, 0.001);
}

test "neighbor finds the pane to the right" {
    var tree = PaneTree.init(t.allocator, 0);
    defer tree.deinit();
    try tree.split(0, .x, 1); // 0 | 1
    var buf: [8]PaneRect = undefined;
    const rect = Rect{ .x = 0, .y = 0, .w = 100, .h = 50 };
    try t.expectEqual(@as(?usize, 1), tree.neighbor(rect, 0, .right, &buf));
    try t.expectEqual(@as(?usize, 0), tree.neighbor(rect, 1, .left, &buf));
    try t.expectEqual(@as(?usize, null), tree.neighbor(rect, 0, .left, &buf));
    try t.expectEqual(@as(?usize, null), tree.neighbor(rect, 0, .up, &buf));
}
