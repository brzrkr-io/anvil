//! Detects what kind of directory the prompt is sitting in, so the prompt can
//! adapt. Pure checks against the filesystem.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    None,
    Zig,
    Node,
    Python,
    Rust,
    Go,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Context {
    pub in_git: bool,
    pub lang: Lang,
    pub has_container: bool,
    pub has_k8s: bool,
}

impl Default for Context {
    fn default() -> Self {
        Self {
            in_git: false,
            lang: Lang::None,
            has_container: false,
            has_k8s: false,
        }
    }
}

/// True if `dir/name` exists.
fn exists(dir: &Path, name: &str) -> bool {
    dir.join(name).exists()
}

/// Inspect `dir` and classify it.
pub fn detect(dir: &Path) -> Context {
    let lang = if exists(dir, "build.zig") {
        Lang::Zig
    } else if exists(dir, "package.json") {
        Lang::Node
    } else if exists(dir, "Cargo.toml") {
        Lang::Rust
    } else if exists(dir, "go.mod") {
        Lang::Go
    } else if exists(dir, "pyproject.toml") || exists(dir, "requirements.txt") {
        Lang::Python
    } else {
        Lang::None
    };

    Context {
        in_git: exists(dir, ".git"),
        lang,
        has_container: exists(dir, "Dockerfile")
            || exists(dir, "docker-compose.yml")
            || exists(dir, "compose.yaml"),
        has_k8s: exists(dir, "kustomization.yaml")
            || exists(dir, "Chart.yaml")
            || exists(dir, "k8s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_tmp() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn detect_classifies_a_zig_git_repo() {
        let tmp = make_tmp();
        let p = tmp.path();
        fs::write(p.join("build.zig"), "").unwrap();
        fs::create_dir(p.join(".git")).unwrap();
        let c = detect(p);
        assert!(c.in_git);
        assert_eq!(c.lang, Lang::Zig);
        assert!(!c.has_container);
    }

    #[test]
    fn detect_finds_a_node_app_with_docker() {
        let tmp = make_tmp();
        let p = tmp.path();
        fs::write(p.join("package.json"), "{}").unwrap();
        fs::write(p.join("Dockerfile"), "").unwrap();
        let c = detect(p);
        assert_eq!(c.lang, Lang::Node);
        assert!(c.has_container);
        assert!(!c.in_git);
    }

    #[test]
    fn detect_on_a_plain_directory_yields_all_false() {
        let tmp = make_tmp();
        let c = detect(tmp.path());
        assert!(!c.in_git);
        assert_eq!(c.lang, Lang::None);
        assert!(!c.has_container);
        assert!(!c.has_k8s);
    }

    #[test]
    fn detect_rust_project_by_cargo_toml() {
        let tmp = make_tmp();
        fs::write(tmp.path().join("Cargo.toml"), "").unwrap();
        let c = detect(tmp.path());
        assert_eq!(c.lang, Lang::Rust);
    }

    #[test]
    fn detect_go_project_by_go_mod() {
        let tmp = make_tmp();
        fs::write(tmp.path().join("go.mod"), "").unwrap();
        let c = detect(tmp.path());
        assert_eq!(c.lang, Lang::Go);
    }

    #[test]
    fn detect_python_project_by_pyproject_toml() {
        let tmp = make_tmp();
        fs::write(tmp.path().join("pyproject.toml"), "").unwrap();
        let c = detect(tmp.path());
        assert_eq!(c.lang, Lang::Python);
    }

    #[test]
    fn detect_python_project_by_requirements_txt() {
        let tmp = make_tmp();
        fs::write(tmp.path().join("requirements.txt"), "").unwrap();
        let c = detect(tmp.path());
        assert_eq!(c.lang, Lang::Python);
    }

    #[test]
    fn detect_k8s_by_kustomization_yaml() {
        let tmp = make_tmp();
        fs::write(tmp.path().join("kustomization.yaml"), "").unwrap();
        let c = detect(tmp.path());
        assert!(c.has_k8s);
    }

    #[test]
    fn detect_k8s_by_chart_yaml() {
        let tmp = make_tmp();
        fs::write(tmp.path().join("Chart.yaml"), "").unwrap();
        let c = detect(tmp.path());
        assert!(c.has_k8s);
    }

    #[test]
    fn detect_k8s_dir() {
        let tmp = make_tmp();
        fs::create_dir(tmp.path().join("k8s")).unwrap();
        let c = detect(tmp.path());
        assert!(c.has_k8s);
    }

    #[test]
    fn detect_container_by_compose_variants() {
        let tmp = make_tmp();
        fs::write(tmp.path().join("docker-compose.yml"), "").unwrap();
        let c = detect(tmp.path());
        assert!(c.has_container);

        let tmp2 = make_tmp();
        fs::write(tmp2.path().join("compose.yaml"), "").unwrap();
        let c2 = detect(tmp2.path());
        assert!(c2.has_container);
    }
}
