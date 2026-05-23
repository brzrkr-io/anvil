//! File-tree model: a flattened, display-ordered list of visible filesystem
//! entries rooted at a directory.  Expand/collapse dirs in-place.  Fixed-size
//! array bounded by `MAX_ENTRIES`.
//!
//! The directory I/O uses `std::fs::read_dir` (no POSIX-specific calls) so
//! this compiles on any target that has a filesystem.

use std::path::{Path, PathBuf};

/// Maximum number of visible entries.
pub const MAX_ENTRIES: usize = 2000;

/// Maximum byte length of a single entry name.
pub const MAX_NAME: usize = 255;

#[derive(Clone, Debug, Default)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub depth: u16,
    pub is_dir: bool,
    pub expanded: bool,
}

pub struct FileTree {
    pub entries: Vec<Entry>,
    pub selected_idx: Option<usize>,
}

impl Default for FileTree {
    fn default() -> Self {
        Self {
            entries: Vec::with_capacity(64),
            selected_idx: None,
        }
    }
}

impl FileTree {
    /// Root the tree at `root_path` and load its immediate children.
    /// The root entry itself is not shown; children are at depth 0.
    pub fn set_root(&mut self, root_path: &Path) {
        self.entries.clear();
        self.selected_idx = None;
        load_children(&mut self.entries, root_path, 0);
    }

    /// Toggle expand/collapse of the entry at visible index `idx`.
    pub fn toggle(&mut self, idx: usize) {
        if idx >= self.entries.len() {
            return;
        }
        if !self.entries[idx].is_dir {
            return;
        }

        if self.entries[idx].expanded {
            // Collapse: remove all following entries whose depth > this entry's depth.
            self.entries[idx].expanded = false;
            let base_depth = self.entries[idx].depth;
            let end = self.entries[idx + 1..]
                .iter()
                .position(|e| e.depth <= base_depth)
                .map(|rel| idx + 1 + rel)
                .unwrap_or(self.entries.len());
            self.entries.drain(idx + 1..end);
        } else {
            self.entries[idx].expanded = true;
            let path = self.entries[idx].path.clone();
            let child_depth = self.entries[idx].depth + 1;

            let mut children: Vec<Entry> = Vec::new();
            collect_children(&mut children, &path, child_depth);

            if children.is_empty() {
                return;
            }

            // Clamp to max_entries.
            let available = (MAX_ENTRIES - self.entries.len()).min(children.len());
            if available == 0 {
                return;
            }
            let insert_at = idx + 1;
            for (i, child) in children.into_iter().take(available).enumerate() {
                self.entries.insert(insert_at + i, child);
            }
        }
    }
}

/// Read the immediate children of `dir_path` sorted dirs-first then files,
/// each group alphabetically.  Appends into `out`.
fn load_children(out: &mut Vec<Entry>, dir_path: &Path, depth: u16) {
    let mut children: Vec<Entry> = Vec::new();
    collect_children(&mut children, dir_path, depth);
    for e in children {
        if out.len() >= MAX_ENTRIES {
            break;
        }
        out.push(e);
    }
}

fn collect_children(out: &mut Vec<Entry>, dir_path: &Path, depth: u16) {
    let rd = match std::fs::read_dir(dir_path) {
        Ok(rd) => rd,
        Err(_) => return,
    };

    let mut dirs: Vec<Entry> = Vec::new();
    let mut files: Vec<Entry> = Vec::new();

    for de in rd.flatten() {
        let name_os = de.file_name();
        let name = name_os.to_string_lossy();
        // Skip hidden files and . / ..
        if name.starts_with('.') {
            continue;
        }
        if name.len() > MAX_NAME {
            continue;
        }
        let is_dir = de.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
        let entry = Entry {
            name: name.into_owned(),
            path: de.path(),
            depth,
            is_dir,
            expanded: false,
        };
        if is_dir {
            dirs.push(entry);
        } else {
            files.push(entry);
        }
    }

    dirs.sort_by(|a, b| a.name.cmp(&b.name));
    files.sort_by(|a, b| a.name.cmp(&b.name));

    out.extend(dirs);
    out.extend(files);
}

// ---------------------------------------------------------------------------
// Tests  (5 Zig tests → 5 Rust tests)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(suffix: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("anvil_filetree_{suffix}_{}", std::process::id()));
        let _ = fs::create_dir_all(&p);
        p
    }

    #[test]
    fn set_root_on_a_temp_directory() {
        let tmp = tmp_dir("setroot");
        let subdir = tmp.join("subdir");
        let file = tmp.join("hello.txt");
        let _ = fs::create_dir(&subdir);
        let _ = fs::write(&file, b"hi");

        let mut tree = FileTree::default();
        tree.set_root(&tmp);

        // 2 entries: subdir (dir) first, hello.txt (file) second.
        assert_eq!(
            tree.entries.len(),
            2,
            "expected 2 entries, got {}",
            tree.entries.len()
        );
        assert_eq!(tree.entries[0].name, "subdir");
        assert!(tree.entries[0].is_dir);
        assert_eq!(tree.entries[1].name, "hello.txt");
        assert!(!tree.entries[1].is_dir);

        let _ = fs::remove_file(&file);
        let _ = fs::remove_dir(&subdir);
        let _ = fs::remove_dir(&tmp);
    }

    #[test]
    fn toggle_expand_then_collapse() {
        let tmp = tmp_dir("toggle");
        let sub = tmp.join("subdir");
        let _ = fs::create_dir(&sub);
        let child = sub.join("child.txt");
        let _ = fs::write(&child, b"");

        let mut tree = FileTree::default();
        tree.set_root(&tmp);
        assert_eq!(tree.entries.len(), 1); // just subdir

        // Expand subdir.
        tree.toggle(0);
        assert_eq!(tree.entries.len(), 2); // subdir + child.txt
        assert!(tree.entries[0].expanded);
        assert_eq!(tree.entries[1].name, "child.txt");
        assert_eq!(tree.entries[1].depth, 1);

        // Collapse subdir.
        tree.toggle(0);
        assert_eq!(tree.entries.len(), 1);
        assert!(!tree.entries[0].expanded);

        let _ = fs::remove_file(&child);
        let _ = fs::remove_dir(&sub);
        let _ = fs::remove_dir(&tmp);
    }

    #[test]
    fn dirs_sort_before_files() {
        let tmp = tmp_dir("sort");
        let _ = fs::create_dir(tmp.join("z_dir"));
        let _ = fs::create_dir(tmp.join("a_dir"));
        let _ = fs::write(tmp.join("z_file.txt"), b"");
        let _ = fs::write(tmp.join("a_file.txt"), b"");

        let mut tree = FileTree::default();
        tree.set_root(&tmp);

        assert_eq!(tree.entries.len(), 4);
        assert!(tree.entries[0].is_dir);
        assert!(tree.entries[1].is_dir);
        // Dirs alphabetically sorted.
        assert!(tree.entries[0].name <= tree.entries[1].name);
        assert!(!tree.entries[2].is_dir);
        assert!(!tree.entries[3].is_dir);

        let _ = fs::remove_dir(tmp.join("z_dir"));
        let _ = fs::remove_dir(tmp.join("a_dir"));
        let _ = fs::remove_file(tmp.join("z_file.txt"));
        let _ = fs::remove_file(tmp.join("a_file.txt"));
        let _ = fs::remove_dir(&tmp);
    }

    #[test]
    fn max_entries_cap_is_enforced() {
        let mut tree = FileTree::default();
        // Root at a known large directory; if unavailable the count stays 0.
        tree.set_root(Path::new("/usr/lib"));
        assert!(tree.entries.len() <= MAX_ENTRIES);
    }

    #[test]
    fn toggle_out_of_bounds_index_is_noop() {
        let mut tree = FileTree::default();
        // Empty tree — any idx is out of bounds.
        tree.toggle(0); // must not panic
        tree.toggle(99); // must not panic
        assert_eq!(tree.entries.len(), 0);
    }

    #[test]
    fn toggle_expand_empty_dir_marks_expanded_but_no_children() {
        let tmp = tmp_dir("emptydir");
        let empty_sub = tmp.join("empty_sub");
        let _ = fs::create_dir(&empty_sub);

        let mut tree = FileTree::default();
        tree.set_root(&tmp);
        // Only the empty_sub directory is an entry.
        assert_eq!(tree.entries.len(), 1);
        assert!(tree.entries[0].is_dir);

        // Toggle expand: no children exist so collect_children returns empty.
        tree.toggle(0);
        // The expand flag is set but no children are inserted.
        assert_eq!(tree.entries.len(), 1);

        let _ = fs::remove_dir(&empty_sub);
        let _ = fs::remove_dir(&tmp);
    }

    #[test]
    fn toggle_on_a_non_dir_is_a_noop() {
        let tmp = tmp_dir("noop");
        let _ = fs::write(tmp.join("f.txt"), b"");

        let mut tree = FileTree::default();
        tree.set_root(&tmp);
        assert_eq!(tree.entries.len(), 1);
        tree.toggle(0); // file, not dir — no-op
        assert_eq!(tree.entries.len(), 1);

        let _ = fs::remove_file(tmp.join("f.txt"));
        let _ = fs::remove_dir(&tmp);
    }

    #[test]
    fn set_root_on_nonexistent_path_yields_empty_tree() {
        let mut tree = FileTree::default();
        tree.set_root(Path::new("/nonexistent_anvil_test_directory_xyz"));
        // collect_children returns early on Err — no entries.
        assert_eq!(0, tree.entries.len());
    }

    #[test]
    fn hidden_files_are_skipped_by_collect_children() {
        let tmp = tmp_dir("hidden");
        let _ = fs::write(tmp.join(".hidden_file"), b"");
        let _ = fs::write(tmp.join("visible.txt"), b"");

        let mut tree = FileTree::default();
        tree.set_root(&tmp);
        // Only visible.txt shows up.
        assert_eq!(1, tree.entries.len());
        assert_eq!("visible.txt", tree.entries[0].name);

        let _ = fs::remove_file(tmp.join(".hidden_file"));
        let _ = fs::remove_file(tmp.join("visible.txt"));
        let _ = fs::remove_dir(&tmp);
    }

    #[test]
    fn overlong_name_files_are_skipped() {
        let tmp = tmp_dir("longname");
        // Create a file with a name exactly 1 char over MAX_NAME (255).
        let long_name = "a".repeat(MAX_NAME + 1);
        let long_path = tmp.join(&long_name);
        match fs::write(&long_path, b"") {
            Ok(()) => {
                let mut tree = FileTree::default();
                tree.set_root(&tmp);
                // The overlong name must not appear.
                assert!(tree.entries.iter().all(|e| e.name.len() <= MAX_NAME));
                let _ = fs::remove_file(&long_path);
            }
            Err(_) => {
                // Some filesystems reject names > 255 bytes at write time — skip.
            }
        }
        let _ = fs::remove_dir(&tmp);
    }
}
