const std = @import("std");

// Minimal regex engine for scrollback search.
// Supported: literals, . (any char), * + ? (greedy), [...]  [^...] classes,
// ^ $ anchors. No alternation. Backtracking matcher; fine for line-length input.
// On compile failure, compile() returns null and the caller falls back to
// literal substring search.

pub const max_nodes = 64;

const NodeKind = enum { lit, any, cls, anchor_start, anchor_end };
const Quant = enum { one, zero_or_one, zero_or_more, one_or_more };

const Node = struct {
    kind: NodeKind = .lit,
    lit: u8 = 0,
    cls_bits: [32]u8 = [_]u8{0} ** 32,
    negate: bool = false,
    quant: Quant = .one,
};

pub const Pattern = struct {
    nodes: [max_nodes]Node = undefined,
    n: usize = 0,

    fn clsMatch(node: *const Node, c: u8) bool {
        const bit = (node.cls_bits[c / 8] >> @intCast(c % 8)) & 1;
        return if (node.negate) bit == 0 else bit != 0;
    }

    fn atomMatch(node: *const Node, c: u8) bool {
        return switch (node.kind) {
            .lit => node.lit == c,
            .any => true,
            .cls => clsMatch(node, c),
            .anchor_start, .anchor_end => false,
        };
    }

    // Match nodes[ni..] against text starting at ti. `text_start` is the
    // original start position for the ^ anchor check.
    // Returns end position (exclusive) on success, or null on failure.
    fn matchHere(self: *const Pattern, ni: usize, text: []const u8, ti: usize, text_start: usize) ?usize {
        if (ni == self.n) return ti;
        const node = &self.nodes[ni];

        if (node.kind == .anchor_start) {
            if (ti != text_start) return null;
            return self.matchHere(ni + 1, text, ti, text_start);
        }
        if (node.kind == .anchor_end) {
            if (ti != text.len) return null;
            return self.matchHere(ni + 1, text, ti, text_start);
        }

        switch (node.quant) {
            .one => {
                if (ti >= text.len) return null;
                if (!atomMatch(node, text[ti])) return null;
                return self.matchHere(ni + 1, text, ti + 1, text_start);
            },
            .zero_or_one => {
                if (ti < text.len and atomMatch(node, text[ti])) {
                    if (self.matchHere(ni + 1, text, ti + 1, text_start)) |e| return e;
                }
                return self.matchHere(ni + 1, text, ti, text_start);
            },
            .zero_or_more => {
                // Greedy: advance as far as possible, then try each end.
                var end = ti;
                while (end < text.len and atomMatch(node, text[end])) end += 1;
                // Try from longest to shortest (greedy).
                while (end >= ti) {
                    if (self.matchHere(ni + 1, text, end, text_start)) |e| return e;
                    if (end == ti) break;
                    end -= 1;
                }
                return null;
            },
            .one_or_more => {
                if (ti >= text.len) return null;
                if (!atomMatch(node, text[ti])) return null;
                var end = ti + 1;
                while (end < text.len and atomMatch(node, text[end])) end += 1;
                while (end >= ti + 1) {
                    if (self.matchHere(ni + 1, text, end, text_start)) |e| return e;
                    if (end == ti + 1) break;
                    end -= 1;
                }
                return null;
            },
        }
    }
};

// Try to match pattern anywhere in `text`. Returns start and length on
// the leftmost (then longest) match, or null if no match.
pub fn search(pat: *const Pattern, text: []const u8) ?struct { start: usize, len: usize } {
    if (pat.n == 0) return null;
    const anchored = pat.nodes[0].kind == .anchor_start;
    const limit: usize = if (anchored) 1 else text.len + 1;
    var start: usize = 0;
    while (start < limit) : (start += 1) {
        if (start > text.len) break;
        if (pat.matchHere(0, text, start, start)) |end| {
            return .{ .start = start, .len = end - start };
        }
    }
    return null;
}

// Compile a pattern string. Returns null on any syntax error.
pub fn compile(src: []const u8) ?Pattern {
    var pat = Pattern{};
    var i: usize = 0;
    while (i < src.len) {
        if (pat.n >= max_nodes) return null;
        var node = Node{};
        switch (src[i]) {
            '^' => {
                node.kind = .anchor_start;
                i += 1;
            },
            '$' => {
                node.kind = .anchor_end;
                i += 1;
            },
            '.' => {
                node.kind = .any;
                i += 1;
            },
            '[' => {
                node.kind = .cls;
                i += 1;
                if (i >= src.len) return null;
                if (src[i] == '^') {
                    node.negate = true;
                    i += 1;
                }
                var first = true;
                while (i < src.len and (first or src[i] != ']')) : (first = false) {
                    const c = src[i];
                    i += 1;
                    // Range: a-z (only when not at end)
                    if (i + 1 < src.len and src[i] == '-' and src[i + 1] != ']') {
                        const lo = c;
                        const hi = src[i + 1];
                        i += 2;
                        if (hi < lo) return null;
                        var k: u8 = lo;
                        while (true) {
                            node.cls_bits[k / 8] |= @as(u8, 1) << @intCast(k % 8);
                            if (k == hi) break;
                            k += 1;
                        }
                    } else {
                        node.cls_bits[c / 8] |= @as(u8, 1) << @intCast(c % 8);
                    }
                }
                if (i >= src.len) return null; // missing ]
                i += 1; // consume ]
            },
            '\\' => {
                i += 1;
                if (i >= src.len) return null;
                node.kind = .lit;
                node.lit = src[i];
                i += 1;
            },
            '*', '+', '?' => return null, // quantifier without preceding atom
            else => {
                node.kind = .lit;
                node.lit = src[i];
                i += 1;
            },
        }
        // Optional quantifier (not valid after anchors).
        if (node.kind != .anchor_start and node.kind != .anchor_end and i < src.len) {
            switch (src[i]) {
                '*' => {
                    node.quant = .zero_or_more;
                    i += 1;
                },
                '+' => {
                    node.quant = .one_or_more;
                    i += 1;
                },
                '?' => {
                    node.quant = .zero_or_one;
                    i += 1;
                },
                else => {},
            }
        }
        pat.nodes[pat.n] = node;
        pat.n += 1;
    }
    return pat;
}

test "literal match" {
    const pat = compile("hello").?;
    const r = search(&pat, "say hello world").?;
    try std.testing.expectEqual(@as(usize, 4), r.start);
    try std.testing.expectEqual(@as(usize, 5), r.len);
}

test "dot matches any" {
    const pat = compile("h.llo").?;
    const r = search(&pat, "xhello").?;
    try std.testing.expectEqual(@as(usize, 1), r.start);
    try std.testing.expectEqual(@as(usize, 5), r.len);
}

test "star quantifier" {
    const pat = compile("ab*c").?;
    const r1 = search(&pat, "ac").?;
    try std.testing.expectEqual(@as(usize, 0), r1.start);
    try std.testing.expectEqual(@as(usize, 2), r1.len);
    const r2 = search(&pat, "abbc").?;
    try std.testing.expectEqual(@as(usize, 4), r2.len);
}

test "plus quantifier" {
    const pat = compile("ab+c").?;
    try std.testing.expect(search(&pat, "ac") == null);
    const r = search(&pat, "abc").?;
    try std.testing.expectEqual(@as(usize, 3), r.len);
}

test "question quantifier" {
    const pat = compile("colou?r").?;
    const r1 = search(&pat, "color").?;
    try std.testing.expectEqual(@as(usize, 5), r1.len);
    const r2 = search(&pat, "colour").?;
    try std.testing.expectEqual(@as(usize, 6), r2.len);
}

test "character class" {
    const pat = compile("[aeiou]").?;
    const r = search(&pat, "xyz e").?;
    try std.testing.expectEqual(@as(usize, 4), r.start);
    try std.testing.expectEqual(@as(usize, 1), r.len);
}

test "negated character class" {
    const pat = compile("[^0-9]+").?;
    const r = search(&pat, "123abc").?;
    try std.testing.expectEqual(@as(usize, 3), r.start);
    try std.testing.expectEqual(@as(usize, 3), r.len);
}

test "character class range" {
    const pat = compile("[a-z]+").?;
    const r = search(&pat, "123abc456").?;
    try std.testing.expectEqual(@as(usize, 3), r.start);
    try std.testing.expectEqual(@as(usize, 3), r.len);
}

test "anchor start" {
    const pat = compile("^foo").?;
    try std.testing.expect(search(&pat, "foo") != null);
    try std.testing.expect(search(&pat, "xfoo") == null);
}

test "anchor end" {
    const pat = compile("bar$").?;
    try std.testing.expect(search(&pat, "foobar") != null);
    try std.testing.expect(search(&pat, "barbaz") == null);
}

test "compile failure returns null" {
    try std.testing.expect(compile("*bad") == null);
    try std.testing.expect(compile("[unclosed") == null);
    try std.testing.expect(compile("\\") == null);
}

test "no match returns null" {
    const pat = compile("xyz").?;
    try std.testing.expect(search(&pat, "hello world") == null);
}

test "empty pattern" {
    const pat = compile("").?;
    try std.testing.expect(search(&pat, "anything") == null);
}
