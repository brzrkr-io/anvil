//! Turns detected context + git info into the ordered Segment list the
//! renderer draws. This is the adaptive core: a segment appears only when the
//! context calls for it.

use crate::context::{Context, Lang};
use crate::git::Info as GitInfo;
use crate::icons::Icon;
use crate::segments::{List, Segment, State};

pub struct Inputs<'a> {
    /// Basename of the working directory.
    pub cwd_base: &'a str,
    pub context: Context,
    pub git_info: Option<GitInfo>,
    pub exit_code: u8,
}

fn lang_text(l: Lang) -> Option<&'static str> {
    match l {
        Lang::None => None,
        Lang::Zig => Some("zig"),
        Lang::Node => Some("node"),
        Lang::Python => Some("python"),
        Lang::Rust => Some("rust"),
        Lang::Go => Some("go"),
    }
}

/// Build the active segment list.
pub fn assemble(input: Inputs<'_>) -> List {
    let mut list = List::new();

    // cwd — always.
    list.add(Segment::new(Icon::Repo, input.cwd_base));

    // git — when in a repo.
    if let Some(g) = input.git_info {
        let (text, state) = if g.dirty > 0 {
            (format!("{} {}", g.branch, g.dirty), State::Warn)
        } else {
            (g.branch.clone(), State::Normal)
        };
        list.add(Segment::with_state(Icon::Branch, text, state));
    }

    // toolchain — when a language is detected.
    if let Some(lt) = lang_text(input.context.lang) {
        list.add(Segment::new(Icon::Toolchain, lt));
    }

    // container / cluster — when present.
    if input.context.has_container {
        list.add(Segment::new(Icon::Container, "docker"));
    }
    if input.context.has_k8s {
        list.add(Segment::new(Icon::Cluster, "k8s"));
    }

    // failure — only on a non-zero exit.
    if input.exit_code != 0 {
        let exit_text = format!("{}", input.exit_code);
        list.add(Segment::with_state(Icon::Err, exit_text, State::Err));
    }

    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Lang;

    #[test]
    fn assemble_clean_repo_shows_cwd_and_branch_only() {
        let list = assemble(Inputs {
            cwd_base: "anvil",
            context: Context {
                in_git: true,
                ..Context::default()
            },
            git_info: Some(GitInfo {
                branch: "main".into(),
                dirty: 0,
                ahead: 0,
                behind: 0,
            }),
            exit_code: 0,
        });
        assert_eq!(list.slice().len(), 2);
        assert_eq!(list.slice()[0].icon, Icon::Repo);
        assert_eq!(list.slice()[0].text, "anvil");
    }

    #[test]
    fn assemble_dirty_repo_marks_git_segment_warn() {
        let list = assemble(Inputs {
            cwd_base: "x",
            context: Context {
                in_git: true,
                ..Context::default()
            },
            git_info: Some(GitInfo {
                branch: "main".into(),
                dirty: 3,
                ahead: 0,
                behind: 0,
            }),
            exit_code: 0,
        });
        assert_eq!(list.slice()[1].state, State::Warn);
        assert!(list.slice()[1].text.contains('3'));
    }

    #[test]
    fn assemble_node_docker_dir_surfaces_toolchain_and_container() {
        let list = assemble(Inputs {
            cwd_base: "app",
            context: Context {
                lang: Lang::Node,
                has_container: true,
                ..Context::default()
            },
            git_info: None,
            exit_code: 0,
        });
        let mut saw_tool = false;
        let mut saw_dk = false;
        for s in list.slice() {
            if s.icon == Icon::Toolchain {
                saw_tool = true;
            }
            if s.icon == Icon::Container {
                saw_dk = true;
            }
        }
        assert!(saw_tool && saw_dk);
    }

    #[test]
    fn assemble_non_zero_exit_adds_err_segment() {
        let list = assemble(Inputs {
            cwd_base: "x",
            context: Context::default(),
            git_info: None,
            exit_code: 127,
        });
        let last = list.slice().last().unwrap();
        assert_eq!(last.icon, Icon::Err);
        assert_eq!(last.text, "127");
    }
}
