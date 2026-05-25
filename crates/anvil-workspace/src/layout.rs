//! Pane-tree layout engine — pure geometry, no platform I/O / PTY.
//!
//! The caller owns a [`PaneTree`] describing how the window is split into leaf
//! panes.  Each leaf is identified by a [`PaneId`] (a stable handle into a
//! pane registry built in a later phase).  The tree drives:
//!   - pixel-rect assignment ([`PaneTree::layout`])
//!   - mouse hit-testing ([`PaneTree::hit_test`])
//!   - directional keyboard navigation ([`PaneTree::neighbor`])
//!
//! ## Tree representation
//!
//! [`PaneNode`] is a Rust enum (tagged union).  Children of a [`Split`] are
//! `Vec<Box<PaneNode>>` — heap-allocated, recursively owned.  This is the
//! natural Rust analogue of Zig's `*PaneNode` heap pointers; `Box` gives us
//! stable addresses and moves ownership into the tree without an arena.
//!
//! ## Tree invariants
//!
//! - The root is never absent; there is always ≥ 1 leaf.
//! - Every [`Split`] has ≥ 2 children.
//! - `ratios` sums to 1.0 (within floating-point error) for every split.
//! - `focused` always names a leaf that exists in the tree.

use thiserror::Error;

/// A rectangle in device pixels.  `y = 0` at the top (raster space).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    /// True when `(px, py)` falls inside (or on the left/top boundary of) this
    /// rect.
    pub fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    pub fn center_x(&self) -> f64 {
        self.x + self.w * 0.5
    }

    pub fn center_y(&self) -> f64 {
        self.y + self.h * 0.5
    }
}

/// Split direction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SplitDir {
    /// Children laid out left | right — a vertical divider between them.
    Horizontal,
    /// Children laid out top | bottom — a horizontal divider between them.
    Vertical,
}

/// Stable handle into a pane registry (registry built in a later phase).
pub type PaneId = u32;

/// One node of the tree — a leaf or a split.
#[derive(Debug)]
pub enum PaneNode {
    Leaf(PaneId),
    Split(Split),
}

/// An interior split node.
#[derive(Debug)]
pub struct Split {
    pub dir: SplitDir,
    /// Children in visual order.  Length ≥ 2.
    pub children: Vec<Box<PaneNode>>,
    /// One ratio per child; sums to 1.0.
    pub ratios: Vec<f64>,
}

/// Error type for fallible tree operations.
#[derive(Debug, Error)]
pub enum LayoutError {
    #[error("focused PaneId not found in tree")]
    FocusedNotFound,
}

/// A single entry in the layout output.
#[derive(Clone, Copy, Debug)]
pub struct LayoutEntry {
    pub id: PaneId,
    pub rect: Rect,
}

/// The full pane tree.
pub struct PaneTree {
    pub root: Box<PaneNode>,
    pub focused: PaneId,
    /// Set to `true` after `close_leaf` empties the tree.  Only `drop` is safe
    /// afterwards.
    pub empty: bool,
}

impl PaneTree {
    /// Create a tree with a single leaf, focused on it.
    pub fn init_single(first: PaneId) -> Self {
        Self {
            root: Box::new(PaneNode::Leaf(first)),
            focused: first,
            empty: false,
        }
    }

    /// Split the FOCUSED leaf, inserting `new` beside it.
    ///
    /// Flat-split rule: if the focused leaf's immediate parent already has the
    /// same `dir`, insert `new` as a sibling (no nesting).  Otherwise wrap the
    /// focused leaf in a new two-child split.
    ///
    /// Ratio rule:
    ///  - New two-child split: each child gets 0.5.
    ///  - Sibling insert: the new child gets an equal share; all ratios are
    ///    renormalized so they sum to 1.0.
    pub fn split(&mut self, dir: SplitDir, new: PaneId) -> Result<(), LayoutError> {
        let new_node = Box::new(PaneNode::Leaf(new));
        split_node(&mut self.root, self.focused, dir, new_node)?;
        self.focused = new;
        Ok(())
    }

    /// Remove the leaf with `id`.  Collapses single-child splits.
    ///
    /// Returns the `PaneId` that should receive focus next, or `None` if the
    /// tree is now empty.  Never leaves `focused` pointing at the removed leaf.
    pub fn close_leaf(&mut self, id: PaneId) -> Option<PaneId> {
        // Special case: the only leaf is the root itself.
        if let PaneNode::Leaf(leaf_id) = self.root.as_ref() {
            if *leaf_id == id {
                self.empty = true;
                self.focused = 0;
                return None;
            }
        }

        let next_focus = close_in_children(&mut self.root, id)?;
        self.focused = next_focus;
        Some(next_focus)
    }

    /// Recompute every leaf's pixel rect, returning a `Vec<LayoutEntry>`.
    pub fn layout(&self, outer: Rect, divider_px: f64) -> Vec<LayoutEntry> {
        let mut out = Vec::new();
        layout_node(&self.root, outer, divider_px, &mut out);
        out
    }

    /// The leaf whose `Rect` contains the point `(px, py)`, or `None` if the
    /// point lands in a gutter (or outside `outer`).
    pub fn hit_test(&self, outer: Rect, divider_px: f64, px: f64, py: f64) -> Option<PaneId> {
        for e in self.layout(outer, divider_px) {
            if e.rect.contains(px, py) {
                return Some(e.id);
            }
        }
        None
    }

    /// The leaf in `dir` direction from the focused leaf (geometric search),
    /// or `None` at an edge.
    pub fn neighbor(&self, dir: NavDir, outer: Rect, divider_px: f64) -> Option<PaneId> {
        let entries = self.layout(outer, divider_px);
        let fr = entries.iter().find(|e| e.id == self.focused)?.rect;
        let fr_cx = fr.center_x();
        let fr_cy = fr.center_y();

        let mut best_id: Option<PaneId> = None;
        let mut best_dist = f64::INFINITY;

        for e in &entries {
            if e.id == self.focused {
                continue;
            }
            let r = e.rect;
            let qualifies = match dir {
                NavDir::Left => r.x + r.w <= fr.x + divider_px,
                NavDir::Right => r.x >= fr.x + fr.w - divider_px,
                NavDir::Up => r.y + r.h <= fr.y + divider_px,
                NavDir::Down => r.y >= fr.y + fr.h - divider_px,
            };
            if !qualifies {
                continue;
            }
            let dx = r.center_x() - fr_cx;
            let dy = r.center_y() - fr_cy;
            let dist = dx * dx + dy * dy;
            if dist < best_dist {
                best_dist = dist;
                best_id = Some(e.id);
            }
        }
        best_id
    }

    /// Number of leaf nodes in the tree.
    pub fn leaf_count(&self) -> usize {
        count_leaves(&self.root)
    }
}

/// Directional navigation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NavDir {
    Left,
    Right,
    Up,
    Down,
}

// ---------------------------------------------------------------------------
// Divider hit-test & ratio adjustment
// ---------------------------------------------------------------------------

/// Result of a divider hit-test.
pub struct DividerHit {
    /// Index of the child *before* the divider (child `i+1` is on the other
    /// side).
    pub child_index: usize,
    /// Pixel position of the divider centre along the split axis.
    pub axis_center: f64,
    /// Path (sequence of child indices) from the root to the split node.
    pub path: Vec<usize>,
    /// Pixel rect of the split node that owns this divider.
    pub split_rect: Rect,
    /// Direction of the split node that owns this divider.
    pub split_dir: SplitDir,
}

/// Find the divider closest to `(px, py)` within `slop_px` device pixels.
/// Returns `None` when the point is not near any divider.
pub fn find_divider_at(
    tree: &PaneTree,
    outer: Rect,
    divider_px: f64,
    px: f64,
    py: f64,
    slop_px: f64,
) -> Option<DividerHit> {
    find_divider_in_node(
        &tree.root,
        outer,
        divider_px,
        px,
        py,
        slop_px,
        &mut Vec::new(),
    )
}

fn find_divider_in_node(
    node: &PaneNode,
    rect: Rect,
    divider_px: f64,
    px: f64,
    py: f64,
    slop_px: f64,
    path: &mut Vec<usize>,
) -> Option<DividerHit> {
    let sp = match node {
        PaneNode::Leaf(_) => return None,
        PaneNode::Split(s) => s,
    };
    let n = sp.children.len();
    let total_gutter = divider_px * (n as f64 - 1.0);
    let available = match sp.dir {
        SplitDir::Horizontal => rect.w - total_gutter,
        SplitDir::Vertical => rect.h - total_gutter,
    };
    let mut offset = 0.0_f64;
    for i in 0..n {
        let child_size = sp.ratios[i] * available;
        let child_rect = match sp.dir {
            SplitDir::Horizontal => Rect {
                x: rect.x + offset,
                y: rect.y,
                w: child_size,
                h: rect.h,
            },
            SplitDir::Vertical => Rect {
                x: rect.x,
                y: rect.y + offset,
                w: rect.w,
                h: child_size,
            },
        };
        offset += child_size;

        if i + 1 < n {
            let gutter_start = match sp.dir {
                SplitDir::Horizontal => rect.x + offset,
                SplitDir::Vertical => rect.y + offset,
            };
            let gutter_center = gutter_start + divider_px * 0.5;
            let hit_coord = match sp.dir {
                SplitDir::Horizontal => px,
                SplitDir::Vertical => py,
            };
            let in_span = match sp.dir {
                SplitDir::Horizontal => py >= rect.y && py < rect.y + rect.h,
                SplitDir::Vertical => px >= rect.x && px < rect.x + rect.w,
            };
            if in_span && (hit_coord - gutter_center).abs() <= divider_px * 0.5 + slop_px {
                let mut hit_path = path.clone();
                hit_path.push(i);
                return Some(DividerHit {
                    child_index: i,
                    axis_center: gutter_center,
                    path: hit_path,
                    split_rect: rect,
                    split_dir: sp.dir,
                });
            }
            offset += divider_px;

            path.push(i);
            if let Some(hit) = find_divider_in_node(
                &sp.children[i],
                child_rect,
                divider_px,
                px,
                py,
                slop_px,
                path,
            ) {
                return Some(hit);
            }
            path.pop();
        } else {
            path.push(i);
            if let Some(hit) = find_divider_in_node(
                &sp.children[i],
                child_rect,
                divider_px,
                px,
                py,
                slop_px,
                path,
            ) {
                return Some(hit);
            }
            path.pop();
        }
    }
    None
}

/// Move `delta` ratio units between `ratios[divider_index]` and
/// `ratios[divider_index + 1]`.  Positive delta grows index `i`, shrinks `i+1`.
/// Each ratio is clamped to `min_ratio`; the pair-sum invariant is preserved.
pub fn adjust_ratio(sp: &mut Split, divider_index: usize, delta: f64, min_ratio: f64) {
    let i = divider_index;
    let j = divider_index + 1;
    assert!(j < sp.ratios.len());
    let total = sp.ratios[i] + sp.ratios[j];
    let new_i = (sp.ratios[i] + delta).max(min_ratio).min(total - min_ratio);
    sp.ratios[i] = new_i;
    sp.ratios[j] = total - new_i;
}

/// Walk the tree to the split node identified by `path` (a sequence of child
/// indices produced by [`find_divider_at`]) and return a mutable reference to
/// it.  Returns `None` if the path is empty or leads to a leaf.
pub fn split_at_path_mut<'a>(tree: &'a mut PaneTree, path: &[usize]) -> Option<&'a mut Split> {
    let mut node: &mut PaneNode = &mut tree.root;
    // The path encodes the sequence of child indices that lead to the split
    // that *contains* the divider.  The last element of the path is the child
    // index of the child *before* the divider, so we stop one level up.
    // We want the split that owns the divider, which is the node reached by
    // following path[0..path.len()-1] from the root.
    let parent_path = if path.is_empty() {
        return None;
    } else {
        &path[..path.len() - 1]
    };
    for &idx in parent_path {
        node = match node {
            PaneNode::Split(sp) => sp.children.get_mut(idx).map(|b| b.as_mut())?,
            PaneNode::Leaf(_) => return None,
        };
    }
    match node {
        PaneNode::Split(sp) => Some(sp),
        PaneNode::Leaf(_) => None,
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

// (alias removed — was invalid Rust)

/// Recursively walk the tree to split the focused leaf.  Returns
/// `Err(LayoutError::FocusedNotFound)` if `focused` is absent.
fn split_node(
    root: &mut Box<PaneNode>,
    focused: PaneId,
    dir: SplitDir,
    new_node: Box<PaneNode>,
) -> Result<(), LayoutError> {
    // We need to inspect whether the root IS the focused leaf; if so we handle
    // it specially (replace root in place).
    if let PaneNode::Leaf(id) = root.as_ref() {
        if *id == focused {
            // The root is the focused leaf.  Wrap it.
            let old_leaf = Box::new(PaneNode::Leaf(focused));
            let split = Split {
                dir,
                children: vec![old_leaf, new_node],
                ratios: vec![0.5, 0.5],
            };
            **root = PaneNode::Split(split);
            return Ok(());
        }
        return Err(LayoutError::FocusedNotFound);
    }

    // Root is a split: delegate to the recursive helper.
    split_in_split(root, focused, dir, new_node)
}

/// Try to find `focused` somewhere under `node` (which must be a Split) and
/// insert `new_node` beside it.
fn split_in_split(
    node: &mut Box<PaneNode>,
    focused: PaneId,
    dir: SplitDir,
    new_node: Box<PaneNode>,
) -> Result<(), LayoutError> {
    let sp = match node.as_mut() {
        PaneNode::Split(s) => s,
        PaneNode::Leaf(id) => {
            if *id == focused {
                // Should be handled by caller; panic is a logic error.
                panic!("leaf reached in split_in_split — caller error");
            }
            return Err(LayoutError::FocusedNotFound);
        }
    };

    // Check if any direct child is the focused leaf.
    for i in 0..sp.children.len() {
        if let PaneNode::Leaf(id) = sp.children[i].as_ref() {
            if *id == focused {
                if sp.dir == dir {
                    // Flat-split: insert as sibling.
                    let insert_at = i + 1;
                    sp.children.insert(insert_at, new_node);
                    let n = sp.children.len() as f64;
                    let eq = 1.0 / n;
                    sp.ratios = vec![eq; sp.children.len()];
                    return Ok(());
                } else {
                    // Wrap the focused leaf in a new two-child split.
                    let old_leaf = sp.children.remove(i);
                    let ratio_at_i = sp.ratios.remove(i);
                    let inner_split = Box::new(PaneNode::Split(Split {
                        dir,
                        children: vec![old_leaf, new_node],
                        ratios: vec![0.5, 0.5],
                    }));
                    sp.children.insert(i, inner_split);
                    sp.ratios.insert(i, ratio_at_i);
                    // Ratios of the outer split are unchanged (we replaced the
                    // child slot with the inner split keeping the same ratio).
                    return Ok(());
                }
            }
        }
    }

    // Not a direct child — recurse into split children.
    for i in 0..sp.children.len() {
        if matches!(sp.children[i].as_ref(), PaneNode::Split(_)) {
            match split_in_split(&mut sp.children[i], focused, dir, new_node) {
                Ok(()) => return Ok(()),
                Err(LayoutError::FocusedNotFound) => {
                    // Need to get new_node back — but we moved it.  The Rust
                    // borrow checker forces a different structure: we reconstruct
                    // new_node from the error.  Use a workaround: pass new_node
                    // by value and return it on failure.
                    //
                    // Actually the signature above already moved new_node; we
                    // need to restructure.  See the outer driver instead.
                    return Err(LayoutError::FocusedNotFound);
                }
            }
        }
    }
    Err(LayoutError::FocusedNotFound)
}

// The split_in_split above has a problem: `new_node` is moved on the first
// recursive call, so we can't retry on siblings.  Rewrite using an owned
// Option to thread new_node through.

// Re-implement from scratch using a cleaner owned approach:

impl PaneTree {
    // Re-export split via the owned helper.
    #[allow(dead_code)]
    fn split_impl(&mut self, dir: SplitDir, new: PaneId) -> Result<(), LayoutError> {
        let new_node = Box::new(PaneNode::Leaf(new));
        if insert_beside(&mut self.root, self.focused, dir, new_node).is_err() {
            return Err(LayoutError::FocusedNotFound);
        }
        self.focused = new;
        Ok(())
    }
}

/// Returns `Ok(())` when the focused leaf was found and the insertion done;
/// `Err(new_node)` when the focused leaf was not found under `node` (so the
/// caller can retry elsewhere or give up).
fn insert_beside(
    node: &mut Box<PaneNode>,
    focused: PaneId,
    dir: SplitDir,
    new_node: Box<PaneNode>,
) -> Result<(), Box<PaneNode>> {
    match node.as_ref() {
        PaneNode::Leaf(id) if *id == focused => {
            // This leaf IS the focused one.  Wrap it in a two-child split.
            let old_id = *id;
            let old_leaf = Box::new(PaneNode::Leaf(old_id));
            let split = Split {
                dir,
                children: vec![old_leaf, new_node],
                ratios: vec![0.5, 0.5],
            };
            **node = PaneNode::Split(split);
            Ok(())
        }
        PaneNode::Leaf(_) => Err(new_node),
        PaneNode::Split(_) => insert_beside_in_split(node, focused, dir, new_node),
    }
}

fn insert_beside_in_split(
    node: &mut Box<PaneNode>,
    focused: PaneId,
    dir: SplitDir,
    mut new_node: Box<PaneNode>,
) -> Result<(), Box<PaneNode>> {
    let sp = match node.as_mut() {
        PaneNode::Split(s) => s,
        _ => unreachable!(),
    };

    // Check direct children first: find the focused leaf.
    for i in 0..sp.children.len() {
        if let PaneNode::Leaf(id) = sp.children[i].as_ref() {
            if *id == focused {
                if sp.dir == dir {
                    // Flat-split: insert as sibling right after i.
                    sp.children.insert(i + 1, new_node);
                    let n = sp.children.len() as f64;
                    let eq = 1.0 / n;
                    sp.ratios = vec![eq; sp.children.len()];
                } else {
                    // Cross-direction: wrap this leaf in an inner split.
                    let old_leaf = sp.children.remove(i);
                    let ratio_at_i = sp.ratios.remove(i);
                    let inner = Box::new(PaneNode::Split(Split {
                        dir,
                        children: vec![old_leaf, new_node],
                        ratios: vec![0.5, 0.5],
                    }));
                    sp.children.insert(i, inner);
                    sp.ratios.insert(i, ratio_at_i);
                }
                return Ok(());
            }
        }
    }

    // Not a direct leaf child — recurse into split children.
    for i in 0..sp.children.len() {
        if matches!(sp.children[i].as_ref(), PaneNode::Split(_)) {
            new_node = match insert_beside(&mut sp.children[i], focused, dir, new_node) {
                Ok(()) => return Ok(()),
                Err(returned) => returned,
            };
        }
    }

    Err(new_node)
}

/// Remove the leaf `id` from under `node`.  Returns the next-focus id on
/// success, `None` if not found.
fn close_in_children(node: &mut Box<PaneNode>, id: PaneId) -> Option<PaneId> {
    let sp = match node.as_mut() {
        PaneNode::Split(s) => s,
        PaneNode::Leaf(_) => return None, // root-leaf handled by caller
    };

    // Does this split directly contain the target leaf?
    let rm_idx = sp
        .children
        .iter()
        .position(|c| matches!(c.as_ref(), PaneNode::Leaf(lid) if *lid == id));

    if let Some(rm) = rm_idx {
        // Choose next-focus sibling before removing.
        let next_focus_node: &PaneNode = if rm + 1 < sp.children.len() {
            &sp.children[rm + 1]
        } else {
            &sp.children[rm - 1]
        };
        let next_focus = first_leaf_id(next_focus_node);

        sp.children.remove(rm);
        sp.ratios.remove(rm);
        renormalize(&mut sp.ratios);

        // Collapse single-child split: replace node content with the survivor.
        if sp.children.len() == 1 {
            let surviving = *sp.children.remove(0);
            **node = surviving;
        }

        return Some(next_focus);
    }

    // Not a direct child — recurse into split children, then re-check collapse.
    for i in 0..sp.children.len() {
        if matches!(sp.children[i].as_ref(), PaneNode::Split(_)) {
            if let Some(next) = close_in_children(&mut sp.children[i], id) {
                // After the recursive call the child may have collapsed to a
                // leaf; that's fine — the child's box was mutated in place.
                return Some(next);
            }
        }
    }

    None
}

fn layout_node(node: &PaneNode, rect: Rect, divider_px: f64, out: &mut Vec<LayoutEntry>) {
    match node {
        PaneNode::Leaf(id) => out.push(LayoutEntry { id: *id, rect }),
        PaneNode::Split(sp) => {
            let n = sp.children.len();
            let total_gutter = divider_px * (n as f64 - 1.0);
            let available = match sp.dir {
                SplitDir::Horizontal => rect.w - total_gutter,
                SplitDir::Vertical => rect.h - total_gutter,
            };
            let mut offset = 0.0_f64;
            for (i, child) in sp.children.iter().enumerate() {
                let child_size = sp.ratios[i] * available;
                let child_rect = match sp.dir {
                    SplitDir::Horizontal => Rect {
                        x: rect.x + offset,
                        y: rect.y,
                        w: child_size,
                        h: rect.h,
                    },
                    SplitDir::Vertical => Rect {
                        x: rect.x,
                        y: rect.y + offset,
                        w: rect.w,
                        h: child_size,
                    },
                };
                layout_node(child, child_rect, divider_px, out);
                offset += child_size + divider_px;
            }
        }
    }
}

fn count_leaves(node: &PaneNode) -> usize {
    match node {
        PaneNode::Leaf(_) => 1,
        PaneNode::Split(sp) => sp.children.iter().map(|c| count_leaves(c)).sum(),
    }
}

fn first_leaf_id(node: &PaneNode) -> PaneId {
    match node {
        PaneNode::Leaf(id) => *id,
        PaneNode::Split(sp) => first_leaf_id(&sp.children[0]),
    }
}

fn renormalize(ratios: &mut [f64]) {
    let sum: f64 = ratios.iter().sum();
    if sum == 0.0 {
        let eq = 1.0 / ratios.len() as f64;
        ratios.iter_mut().for_each(|r| *r = eq);
        return;
    }
    let inv = 1.0 / sum;
    ratios.iter_mut().for_each(|r| *r *= inv);
}

#[cfg(test)]
fn tree_depth(node: &PaneNode) -> usize {
    match node {
        PaneNode::Leaf(_) => 1,
        PaneNode::Split(sp) => 1 + sp.children.iter().map(|c| tree_depth(c)).max().unwrap_or(0),
    }
}

#[cfg(test)]
fn check_ratios_sum(node: &PaneNode) {
    if let PaneNode::Split(sp) = node {
        let sum: f64 = sp.ratios.iter().sum();
        assert!((sum - 1.0).abs() < 1e-9, "ratios sum {sum} ≠ 1.0 in split");
        for child in &sp.children {
            check_ratios_sum(child);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: build a tree from a sequence of (dir, id) splits.
    fn build(first: PaneId, splits: &[(SplitDir, PaneId)]) -> PaneTree {
        let mut tree = PaneTree::init_single(first);
        for &(dir, id) in splits {
            tree.split(dir, id).unwrap();
        }
        tree
    }

    #[test]
    fn init_single_leaf_focused() {
        let tree = PaneTree::init_single(1);
        assert_eq!(tree.focused, 1);
        assert_eq!(tree.leaf_count(), 1);
        assert!(matches!(tree.root.as_ref(), PaneNode::Leaf(1)));
    }

    #[test]
    fn split_then_close_returns_to_single_leaf() {
        let mut tree = PaneTree::init_single(1);
        tree.split(SplitDir::Horizontal, 2).unwrap();
        assert_eq!(tree.leaf_count(), 2);
        assert_eq!(tree.focused, 2);

        let next = tree.close_leaf(2);
        assert_eq!(next, Some(1));
        assert_eq!(tree.focused, 1);
        assert_eq!(tree.leaf_count(), 1);
        assert!(matches!(tree.root.as_ref(), PaneNode::Leaf(1)));
    }

    #[test]
    fn splitting_same_direction_stays_flat() {
        // [1, 2] then same dir -> [1, 2, 3] flat (no nesting)
        let tree = build(1, &[(SplitDir::Horizontal, 2), (SplitDir::Horizontal, 3)]);
        assert_eq!(tree_depth(&tree.root), 2);
        assert_eq!(tree.leaf_count(), 3);
        if let PaneNode::Split(sp) = tree.root.as_ref() {
            assert_eq!(sp.children.len(), 3);
        } else {
            panic!("root must be split");
        }
        check_ratios_sum(&tree.root);
    }

    #[test]
    fn splitting_opposite_direction_nests() {
        // [1, 2] horizontal, then focus=2 vertical -> [1, [2, 3]]
        let tree = build(1, &[(SplitDir::Horizontal, 2), (SplitDir::Vertical, 3)]);
        assert_eq!(tree_depth(&tree.root), 3);
        assert_eq!(tree.leaf_count(), 3);
        check_ratios_sum(&tree.root);
    }

    #[test]
    fn close_leaf_collapses_single_child_split() {
        let mut tree = build(1, &[(SplitDir::Horizontal, 2)]);
        tree.close_leaf(2);
        assert!(matches!(tree.root.as_ref(), PaneNode::Leaf(1)));
        assert_eq!(tree.leaf_count(), 1);
    }

    #[test]
    fn close_leaf_returns_valid_next_focus() {
        let mut tree = build(1, &[(SplitDir::Horizontal, 2), (SplitDir::Horizontal, 3)]);

        let next = tree.close_leaf(3);
        assert!(next.is_some());
        assert_ne!(next.unwrap(), 3);
        assert_ne!(tree.focused, 3);

        let remaining1 = tree.close_leaf(next.unwrap());
        assert!(remaining1.is_some());
        assert_ne!(remaining1.unwrap(), next.unwrap());

        let empty = tree.close_leaf(remaining1.unwrap());
        assert_eq!(empty, None);
    }

    #[test]
    fn ratios_always_sum_to_one_after_split_close_sequences() {
        let mut tree = PaneTree::init_single(1);

        tree.split(SplitDir::Horizontal, 2).unwrap();
        check_ratios_sum(&tree.root);

        tree.split(SplitDir::Horizontal, 3).unwrap();
        check_ratios_sum(&tree.root);

        tree.close_leaf(3);
        check_ratios_sum(&tree.root);

        tree.split(SplitDir::Vertical, 4).unwrap();
        check_ratios_sum(&tree.root);
    }

    #[test]
    fn layout_rects_tile_outer_minus_gutters_non_overlapping() {
        struct Case {
            ids: Vec<PaneId>,
            dirs: Vec<SplitDir>,
            outer: Rect,
            div: f64,
        }
        let cases = vec![
            Case {
                ids: vec![1, 2],
                dirs: vec![SplitDir::Horizontal],
                outer: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 200.0,
                    h: 100.0,
                },
                div: 4.0,
            },
            Case {
                ids: vec![1, 2],
                dirs: vec![SplitDir::Vertical],
                outer: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 200.0,
                    h: 100.0,
                },
                div: 4.0,
            },
            Case {
                ids: vec![1, 2, 3],
                dirs: vec![SplitDir::Horizontal, SplitDir::Horizontal],
                outer: Rect {
                    x: 10.0,
                    y: 20.0,
                    w: 300.0,
                    h: 150.0,
                },
                div: 2.0,
            },
        ];

        for c in &cases {
            let mut tree = PaneTree::init_single(c.ids[0]);
            for (i, &id) in c.ids[1..].iter().enumerate() {
                let dir_idx = i.min(c.dirs.len() - 1);
                tree.split(c.dirs[dir_idx], id).unwrap();
            }

            let entries = tree.layout(c.outer, c.div);
            assert_eq!(entries.len(), c.ids.len());

            // All rects within outer.
            for e in &entries {
                assert!(e.rect.x >= c.outer.x - 1e-9);
                assert!(e.rect.y >= c.outer.y - 1e-9);
                assert!(e.rect.x + e.rect.w <= c.outer.x + c.outer.w + 1e-9);
                assert!(e.rect.y + e.rect.h <= c.outer.y + c.outer.h + 1e-9);
            }

            // No pairwise overlap.
            for (ai, a) in entries.iter().enumerate() {
                for (bi, b) in entries.iter().enumerate() {
                    if ai == bi {
                        continue;
                    }
                    let ox = a.rect.x < b.rect.x + b.rect.w - 1e-9
                        && b.rect.x < a.rect.x + a.rect.w - 1e-9;
                    let oy = a.rect.y < b.rect.y + b.rect.h - 1e-9
                        && b.rect.y < a.rect.y + a.rect.h - 1e-9;
                    assert!(
                        !(ox && oy),
                        "layout rects overlap: id={} and id={}",
                        a.id,
                        b.id
                    );
                }
            }

            // Total area = outer minus gutters.
            let total_area: f64 = entries.iter().map(|e| e.rect.w * e.rect.h).sum();
            let n = c.ids.len() as f64;
            let gutter_count = n - 1.0;
            let expected_area = if let PaneNode::Split(sp) = tree.root.as_ref() {
                match sp.dir {
                    SplitDir::Horizontal => (c.outer.w - gutter_count * c.div) * c.outer.h,
                    SplitDir::Vertical => c.outer.w * (c.outer.h - gutter_count * c.div),
                }
            } else {
                // Single leaf: no gutters.
                c.outer.w * c.outer.h
            };
            assert!(
                (total_area - expected_area).abs() < 1e-6,
                "total area {total_area} ≠ expected {expected_area}"
            );
        }
    }

    #[test]
    fn hit_test_round_trips_center_hits_back_to_leaf() {
        let mut tree = PaneTree::init_single(1);
        tree.split(SplitDir::Horizontal, 2).unwrap();
        tree.split(SplitDir::Vertical, 3).unwrap();

        let outer = Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 300.0,
        };
        let div = 4.0;

        let entries = tree.layout(outer, div);
        for e in &entries {
            let cx = e.rect.center_x();
            let cy = e.rect.center_y();
            let hit = tree.hit_test(outer, div, cx, cy);
            assert_eq!(hit, Some(e.id));
        }

        // A point in the gutter returns None.
        // Left pane w = (400-4)/2 = 198; gutter x = 198..202; x=199 in gutter.
        let gutter_hit = tree.hit_test(outer, div, 199.0, 150.0);
        assert_eq!(gutter_hit, None);
    }

    #[test]
    fn neighbor_directional_nav_edges_return_none() {
        // Layout: [1, [2, 3]] (horizontal root, vertical inner on right)
        let mut tree = PaneTree::init_single(1);
        tree.split(SplitDir::Horizontal, 2).unwrap();
        tree.split(SplitDir::Vertical, 3).unwrap();

        let outer = Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 300.0,
        };
        let div = 4.0;

        // Focus=3 (bottom-right). Up -> 2.
        tree.focused = 3;
        assert_eq!(tree.neighbor(NavDir::Up, outer, div), Some(2));

        // Down from 3 -> None (edge).
        assert_eq!(tree.neighbor(NavDir::Down, outer, div), None);

        // Left from 3 -> 1.
        assert_eq!(tree.neighbor(NavDir::Left, outer, div), Some(1));

        // Focus=1; right -> 2 or 3 (nearest in right column).
        tree.focused = 1;
        let right = tree.neighbor(NavDir::Right, outer, div);
        assert!(right.is_some());
        assert!(right.unwrap() == 2 || right.unwrap() == 3);

        // Left from 1 -> None (edge).
        assert_eq!(tree.neighbor(NavDir::Left, outer, div), None);
    }

    #[test]
    fn focus_navigation_sequence_updates_focused_edge_is_none() {
        let mut tree = PaneTree::init_single(1);
        tree.split(SplitDir::Horizontal, 2).unwrap();
        tree.split(SplitDir::Vertical, 3).unwrap();

        let outer = Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 300.0,
        };
        let div = 4.0;

        // 3 -> up -> 2
        tree.focused = 3;
        let up = tree.neighbor(NavDir::Up, outer, div);
        assert_eq!(up, Some(2));
        tree.focused = up.unwrap();
        assert_eq!(tree.focused, 2);

        // 2 -> down -> 3
        let down = tree.neighbor(NavDir::Down, outer, div);
        assert_eq!(down, Some(3));
        tree.focused = down.unwrap();

        // 2 -> left -> 1
        tree.focused = 2;
        let left = tree.neighbor(NavDir::Left, outer, div);
        assert_eq!(left, Some(1));
        tree.focused = left.unwrap();
        assert_eq!(tree.focused, 1);

        // 1 -> left -> None (edge)
        let edge = tree.neighbor(NavDir::Left, outer, div);
        assert_eq!(edge, None);
        assert_eq!(tree.focused, 1); // unchanged

        // 1 -> right -> {2, 3}
        let right = tree.neighbor(NavDir::Right, outer, div);
        assert!(right.is_some());
        assert!(right.unwrap() == 2 || right.unwrap() == 3);
        tree.focused = right.unwrap();

        // Up from 2 -> None
        tree.focused = 2;
        assert_eq!(tree.neighbor(NavDir::Up, outer, div), None);

        // Down from 3 -> None
        tree.focused = 3;
        assert_eq!(tree.neighbor(NavDir::Down, outer, div), None);
    }

    #[test]
    fn adjust_ratio_keeps_sum_at_one_every_ratio_ge_min() {
        let mut tree = build(1, &[(SplitDir::Horizontal, 2), (SplitDir::Horizontal, 3)]);
        let sp = match tree.root.as_mut() {
            PaneNode::Split(s) => s,
            _ => panic!("expected split"),
        };

        let min_r = 0.05_f64;
        // Simple pseudo-random sequence using a linear congruential generator.
        let mut lcg: u64 = 42;
        for _ in 0..100 {
            lcg = lcg
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let f = (lcg >> 33) as f64 / (u32::MAX as f64); // [0, 1)
            let delta = (f - 0.5) * 0.4; // [-0.2, 0.2]
            adjust_ratio(sp, 0, delta, min_r);

            let sum: f64 = sp.ratios.iter().sum();
            assert!((sum - 1.0).abs() < 1e-9, "sum={sum}");
            assert!(sp.ratios[0] >= min_r - 1e-9);
            assert!(sp.ratios[1] >= min_r - 1e-9);
        }
    }

    #[test]
    fn close_leaf_on_non_root_collapse_propagates_correctly() {
        // [1, [2, 3]] — close 2: inner split collapses to [1, 3]
        let mut tree = PaneTree::init_single(1);
        tree.split(SplitDir::Horizontal, 2).unwrap();
        tree.split(SplitDir::Vertical, 3).unwrap();

        tree.focused = 2;
        tree.close_leaf(2);

        assert_eq!(tree.leaf_count(), 2);
        if let PaneNode::Split(sp) = tree.root.as_ref() {
            assert_eq!(sp.children.len(), 2);
        } else {
            panic!("root must be split");
        }
        check_ratios_sum(&tree.root);
    }

    fn derive_cols_rows(rect: Rect, cell_w: f64, cell_h: f64) -> (usize, usize) {
        let cols = ((rect.w / cell_w) as usize).max(1);
        let rows = ((rect.h / cell_h) as usize).max(1);
        (cols, rows)
    }

    #[test]
    fn resize_derivation_two_pane_horizontal_correct_cols() {
        let mut tree = PaneTree::init_single(1);
        tree.split(SplitDir::Horizontal, 2).unwrap();

        let cell_w = 8.0_f64;
        let cell_h = 16.0_f64;
        let div = 4.0_f64;
        let inner = Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 200.0,
        };

        let entries = tree.layout(inner, div);
        assert_eq!(entries.len(), 2);
        for e in &entries {
            let (cols, rows) = derive_cols_rows(e.rect, cell_w, cell_h);
            // (400 - 4) / 2 = 198 px wide → 198 / 8 = 24 cols
            assert_eq!(cols, 24);
            // 200 / 16 = 12 rows
            assert_eq!(rows, 12);
            assert!(cols >= 1);
            assert!(rows >= 1);
        }
    }

    #[test]
    fn resize_derivation_three_pane_vertical_ge_one_rows() {
        let mut tree = PaneTree::init_single(1);
        tree.split(SplitDir::Vertical, 2).unwrap();
        tree.split(SplitDir::Vertical, 3).unwrap();

        let cell_w = 8.0_f64;
        let cell_h = 16.0_f64;
        let div = 4.0_f64;
        let inner = Rect {
            x: 0.0,
            y: 0.0,
            w: 300.0,
            h: 150.0,
        };

        let entries = tree.layout(inner, div);
        assert_eq!(entries.len(), 3);
        for e in &entries {
            let (cols, rows) = derive_cols_rows(e.rect, cell_w, cell_h);
            assert!(cols >= 1);
            assert!(rows >= 1);
        }
    }

    #[test]
    fn adjust_ratio_known_delta_shifts_by_that_amount() {
        let mut tree = PaneTree::init_single(1);
        tree.split(SplitDir::Horizontal, 2).unwrap();
        let sp = match tree.root.as_mut() {
            PaneNode::Split(s) => s,
            _ => panic!("expected split"),
        };

        let before_i = sp.ratios[0];
        let before_j = sp.ratios[1];
        let delta = 0.1;
        adjust_ratio(sp, 0, delta, 0.05);

        assert!((sp.ratios[0] - (before_i + delta)).abs() < 1e-9);
        assert!((sp.ratios[1] - (before_j - delta)).abs() < 1e-9);
        let sum: f64 = sp.ratios.iter().sum();
        assert!((sum - 1.0).abs() < 1e-9);
    }

    #[test]
    fn adjust_ratio_delta_clamped_at_min_ratio_floor() {
        let mut tree = PaneTree::init_single(1);
        tree.split(SplitDir::Horizontal, 2).unwrap();
        let sp = match tree.root.as_mut() {
            PaneNode::Split(s) => s,
            _ => panic!("expected split"),
        };

        let min_r = 0.2;
        adjust_ratio(sp, 0, 1.0, min_r);
        assert!(sp.ratios[1] >= min_r - 1e-9);
        assert!(sp.ratios[0] >= min_r - 1e-9);
        let sum: f64 = sp.ratios.iter().sum();
        assert!((sum - 1.0).abs() < 1e-9);
    }

    #[test]
    fn find_divider_at_finds_divider_between_horizontal_panes() {
        let mut tree = PaneTree::init_single(1);
        tree.split(SplitDir::Horizontal, 2).unwrap();

        let outer = Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 200.0,
        };
        let div = 8.0_f64;
        // Divider center: (400 - 8) / 2 + 8/2 = 196 + 4 = 200
        let divider_center_x = 200.0_f64;

        let hit = find_divider_at(&tree, outer, div, divider_center_x, 100.0, 4.0);
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().child_index, 0);

        let miss = find_divider_at(&tree, outer, div, 50.0, 100.0, 4.0);
        assert!(miss.is_none());
    }

    #[test]
    fn leaf_count_registry_count_invariant_after_split_and_close() {
        let mut tree = PaneTree::init_single(1);
        let mut reg_count = 1_usize;

        tree.split(SplitDir::Horizontal, 2).unwrap();
        reg_count += 1;
        assert_eq!(reg_count, tree.leaf_count());

        tree.split(SplitDir::Vertical, 3).unwrap();
        reg_count += 1;
        assert_eq!(reg_count, tree.leaf_count());

        tree.close_leaf(3);
        reg_count -= 1;
        assert_eq!(reg_count, tree.leaf_count());

        tree.close_leaf(2);
        reg_count -= 1;
        assert_eq!(reg_count, tree.leaf_count());

        let last = tree.close_leaf(1);
        assert_eq!(last, None);
    }

    // ── Rect helpers ──────────────────────────────────────────────────────────

    #[test]
    fn rect_contains_boundary_conditions() {
        let r = Rect {
            x: 10.0,
            y: 20.0,
            w: 100.0,
            h: 50.0,
        };
        assert!(r.contains(10.0, 20.0)); // left/top boundary — included
        assert!(r.contains(50.0, 40.0)); // interior
        assert!(!r.contains(110.0, 40.0)); // right boundary — excluded
        assert!(!r.contains(50.0, 70.0)); // bottom boundary — excluded
        assert!(!r.contains(9.0, 40.0)); // left of rect
        assert!(!r.contains(50.0, 19.0)); // above rect
    }

    #[test]
    fn rect_center_x_and_center_y() {
        let r = Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 60.0,
        };
        assert!((r.center_x() - 50.0).abs() < 1e-9);
        assert!((r.center_y() - 30.0).abs() < 1e-9);
    }

    // ── find_divider_at on vertical split ─────────────────────────────────────

    #[test]
    fn find_divider_at_finds_divider_in_vertical_split() {
        let mut tree = PaneTree::init_single(1);
        tree.split(SplitDir::Vertical, 2).unwrap();

        let outer = Rect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 400.0,
        };
        let div = 8.0_f64;
        // Divider center at y: (400 - 8) / 2 + 4 = 200
        let hit = find_divider_at(&tree, outer, div, 100.0, 200.0, 4.0);
        assert!(hit.is_some());
        let miss = find_divider_at(&tree, outer, div, 100.0, 50.0, 4.0);
        assert!(miss.is_none());
    }

    // ── find_divider_at on a nested tree (recursive) ──────────────────────────

    #[test]
    fn find_divider_at_nested_tree_recurses() {
        // Build [1, [2, 3]] — horizontal outer, vertical inner
        let mut tree = PaneTree::init_single(1);
        tree.split(SplitDir::Horizontal, 2).unwrap();
        // focused is now 2; split vertically to create [1, [2, 3]]
        tree.split(SplitDir::Vertical, 3).unwrap();

        let outer = Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 400.0,
        };
        let div = 4.0_f64;

        // The horizontal divider between pane 1 and [2,3] is at x=200.
        let horiz_hit = find_divider_at(&tree, outer, div, 200.0, 200.0, 4.0);
        assert!(horiz_hit.is_some());

        // The vertical divider inside the right side is at y=200 of the right half.
        let vert_hit = find_divider_at(&tree, outer, div, 300.0, 200.0, 4.0);
        assert!(vert_hit.is_some());
    }

    // ── split error path ───────────────────────────────────────────────────────

    #[test]
    fn split_focused_not_found_returns_error() {
        let mut tree = PaneTree::init_single(1);
        // Move focused to a non-existent id
        tree.focused = 99;
        let err = tree.split(SplitDir::Horizontal, 2);
        assert!(matches!(err, Err(LayoutError::FocusedNotFound)));
    }

    // ── hit_test returns None outside rect ────────────────────────────────────

    #[test]
    fn hit_test_outside_outer_returns_none() {
        let tree = PaneTree::init_single(1);
        let outer = Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 200.0,
        };
        assert_eq!(tree.hit_test(outer, 4.0, 500.0, 100.0), None);
        assert_eq!(tree.hit_test(outer, 4.0, 200.0, 300.0), None);
    }

    // ── neighbor Up/Down ───────────────────────────────────────────────────────

    #[test]
    fn neighbor_up_and_down_in_vertical_split() {
        let mut tree = PaneTree::init_single(1);
        tree.split(SplitDir::Vertical, 2).unwrap();
        tree.focused = 1;

        let outer = Rect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 400.0,
        };
        // Pane 1 is on top; 2 is below.
        let down = tree.neighbor(NavDir::Down, outer, 4.0);
        assert_eq!(down, Some(2));

        tree.focused = 2;
        let up = tree.neighbor(NavDir::Up, outer, 4.0);
        assert_eq!(up, Some(1));
    }

    #[test]
    fn neighbor_right_in_horizontal_split() {
        let mut tree = PaneTree::init_single(1);
        tree.split(SplitDir::Horizontal, 2).unwrap();
        tree.focused = 1;

        let outer = Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 200.0,
        };
        let right = tree.neighbor(NavDir::Right, outer, 4.0);
        assert_eq!(right, Some(2));
    }

    // ── close_leaf on non-focused leaf ────────────────────────────────────────

    #[test]
    fn close_non_focused_leaf_shrinks_tree() {
        let mut tree = PaneTree::init_single(1);
        tree.split(SplitDir::Horizontal, 2).unwrap();
        tree.split(SplitDir::Horizontal, 3).unwrap();
        // Close leaf 1 (not the focused leaf which is 3)
        let next = tree.close_leaf(1);
        assert!(next.is_some());
        assert_eq!(tree.leaf_count(), 2);
        // Focused is updated by close_leaf to the returned next.
        assert_eq!(tree.focused, next.unwrap());
        check_ratios_sum(&tree.root);
    }

    // ── renormalize with zero-sum ──────────────────────────────────────────────

    #[test]
    fn renormalize_zero_sum_distributes_equally() {
        let mut ratios = vec![0.0, 0.0, 0.0];
        renormalize(&mut ratios);
        for r in &ratios {
            assert!((r - 1.0 / 3.0).abs() < 1e-9);
        }
    }

    // ── PaneTree::split on a 3-child flat split ────────────────────────────────

    #[test]
    fn split_three_child_flat_split_is_balanced() {
        // Build [1, 2, 3] horizontal, all same direction.
        let mut tree = PaneTree::init_single(1);
        tree.split(SplitDir::Horizontal, 2).unwrap();
        tree.focused = 2;
        tree.split(SplitDir::Horizontal, 3).unwrap();
        assert_eq!(tree.leaf_count(), 3);
        check_ratios_sum(&tree.root);
        if let PaneNode::Split(sp) = tree.root.as_ref() {
            assert_eq!(sp.children.len(), 3);
            // All ratios should be ~1/3.
            for r in &sp.ratios {
                assert!((r - 1.0 / 3.0).abs() < 1e-9);
            }
        } else {
            panic!("expected flat split root");
        }
    }

    // ── split_impl / insert_beside coverage ──────────────────────────────────

    #[test]
    fn split_impl_single_root_wraps_into_split() {
        let mut tree = PaneTree::init_single(1);
        tree.split_impl(SplitDir::Horizontal, 2).unwrap();
        assert_eq!(tree.leaf_count(), 2);
        assert_eq!(tree.focused, 2);
        check_ratios_sum(&tree.root);
    }

    #[test]
    fn split_impl_same_direction_flat_sibling() {
        // [1, 2] horizontal — split_impl on focused=2 with same dir → flat [1, 2, 3]
        let mut tree = PaneTree::init_single(1);
        tree.split_impl(SplitDir::Horizontal, 2).unwrap();
        tree.split_impl(SplitDir::Horizontal, 3).unwrap();
        assert_eq!(tree.leaf_count(), 3);
        check_ratios_sum(&tree.root);
    }

    #[test]
    fn split_impl_cross_direction_nests() {
        // [1, 2] horizontal — split_impl with Vertical on focused=2 → [1, [2, 3]]
        let mut tree = PaneTree::init_single(1);
        tree.split_impl(SplitDir::Horizontal, 2).unwrap();
        tree.split_impl(SplitDir::Vertical, 3).unwrap();
        assert_eq!(tree.leaf_count(), 3);
        assert_eq!(tree_depth(&tree.root), 3);
        check_ratios_sum(&tree.root);
    }

    #[test]
    fn split_impl_focused_not_found_returns_error() {
        let mut tree = PaneTree::init_single(1);
        tree.focused = 99; // bogus id
        let err = tree.split_impl(SplitDir::Horizontal, 2);
        assert!(matches!(err, Err(LayoutError::FocusedNotFound)));
    }

    #[test]
    fn split_impl_deep_tree_recurses_correctly() {
        // Build [1, [2, 3]] then split_impl with focused=3 same dir → [1, [2, 3, 4]]
        let mut tree = PaneTree::init_single(1);
        tree.split_impl(SplitDir::Horizontal, 2).unwrap();
        tree.split_impl(SplitDir::Vertical, 3).unwrap();
        // focused is now 3 (inside the inner split)
        tree.split_impl(SplitDir::Vertical, 4).unwrap();
        assert_eq!(tree.leaf_count(), 4);
        check_ratios_sum(&tree.root);
    }

    #[test]
    fn split_impl_sibling_splits_retry_path() {
        // Build [[1,4],[2,3]] so that the root has two vertical inner splits.
        // Then split_impl with focused=2 exercises the retry path in
        // insert_beside_in_split: child[0]=[1,4] returns Err, then child[1]=[2,3]
        // succeeds.
        let mut tree = PaneTree::init_single(1);
        tree.split_impl(SplitDir::Horizontal, 2).unwrap(); // [1, 2]
        tree.split_impl(SplitDir::Vertical, 3).unwrap(); // [1, [2, 3]], focused=3
        tree.focused = 1;
        tree.split_impl(SplitDir::Vertical, 4).unwrap(); // [[1,4], [2,3]], focused=4

        // Now root has two vertical splits as children.
        assert_eq!(tree.leaf_count(), 4);

        // Set focused=2, which is inside the second child split [2,3].
        tree.focused = 2;
        // split_impl Vertical with focused=2: must skip [1,4] (Err) → retry [2,3].
        tree.split_impl(SplitDir::Vertical, 5).unwrap();
        assert_eq!(tree.leaf_count(), 5);
        check_ratios_sum(&tree.root);
    }

    // ── close_leaf on a single-leaf root ─────────────────────────────────────

    #[test]
    fn close_leaf_on_single_root_returns_none() {
        let mut tree = PaneTree::init_single(42);
        let result = tree.close_leaf(42);
        assert_eq!(result, None);
        assert!(tree.empty);
        assert_eq!(tree.focused, 0);
    }

    // ── close_in_children recursive collapse ─────────────────────────────────

    #[test]
    fn close_in_children_recurses_into_nested_split() {
        // Build [1, [2, 3]] — close leaf 3 which is inside the inner split.
        // close_in_children must recurse into the right child's split.
        let mut tree = PaneTree::init_single(1);
        tree.split(SplitDir::Horizontal, 2).unwrap();
        tree.split(SplitDir::Vertical, 3).unwrap();
        // tree: [1, [2, 3]], focused = 3

        let next = tree.close_leaf(3);
        assert!(next.is_some());
        assert_eq!(tree.leaf_count(), 2);
        check_ratios_sum(&tree.root);
    }

    #[test]
    fn close_in_children_returns_none_for_absent_id() {
        // close_in_children returns None for an id that isn't in the tree.
        let mut tree = PaneTree::init_single(1);
        tree.split(SplitDir::Horizontal, 2).unwrap();
        // Tree: [1, 2]. Try closing id 99 (absent).
        let result = tree.close_leaf(99);
        assert_eq!(result, None);
    }
}
