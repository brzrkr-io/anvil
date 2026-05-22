use std::path::Path;

use anvil_control::AiSessionBroker;

fn main() {
    prepare_caldera_session_on_repo_open();
}

fn prepare_caldera_session_on_repo_open() {
    let Ok(cwd) = std::env::current_dir() else {
        return;
    };
    if !is_caldera_enabled_repo(&cwd) {
        return;
    }

    let agent = std::env::var("ANVIL_CALDERA_AGENT").unwrap_or_else(|_| "codex".to_string());
    let task = std::env::var("ANVIL_CALDERA_TASK")
        .unwrap_or_else(|_| "Open this repo in Anvil and prepare safe AI context".to_string());

    match AiSessionBroker::localhost().prepare_repo_session(task, agent) {
        Ok(session) => {
            eprintln!(
                "anvil: prepared Caldera session {} ({})",
                session.session_id, session.handoff_path
            );
        }
        Err(error) => {
            eprintln!("anvil: Caldera session preparation skipped: {error}");
        }
    }
}

fn is_caldera_enabled_repo(cwd: &Path) -> bool {
    cwd.join(".caldera/project.json").exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_caldera_enabled_repo() {
        let dir = std::env::temp_dir().join(format!("anvil-caldera-detect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!is_caldera_enabled_repo(&dir));

        std::fs::create_dir_all(dir.join(".caldera")).unwrap();
        std::fs::write(dir.join(".caldera/project.json"), "{}").unwrap();

        assert!(is_caldera_enabled_repo(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
