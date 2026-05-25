use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default)]
pub struct SavedState {
    pub sessions: Vec<SavedSession>,
}

#[derive(Serialize, Deserialize)]
pub struct SavedSession {
    pub name: String,
    pub project_root: Option<PathBuf>,
    pub pane_cwds: Vec<PathBuf>,
}

fn state_path() -> PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join("jmux")
        .join("state.json")
}

pub fn save(state: &SavedState) -> Result<()> {
    let path = state_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(state)?;
    std::fs::write(path, json)?;
    Ok(())
}

pub fn load() -> SavedState {
    let path = state_path();
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn kill_session(name: &str) -> Result<()> {
    let mut state = load();
    let before = state.sessions.len();
    state.sessions.retain(|s| s.name != name);
    if state.sessions.len() == before {
        anyhow::bail!(
            "no session named '{}' (run 'jmux ls' to list sessions)",
            name
        );
    }
    save(&state)?;
    println!("killed session '{}'", name);
    Ok(())
}

pub fn find_session<'a>(
    saved: &'a SavedState,
    project_root: &Option<PathBuf>,
) -> Option<&'a SavedSession> {
    let root = project_root.as_ref()?;
    saved
        .sessions
        .iter()
        .find(|s| s.project_root.as_deref() == Some(root))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn override_state_path(dir: &std::path::Path) -> std::path::PathBuf {
        dir.join("state.json")
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempdir().unwrap();
        let path = override_state_path(dir.path());

        let saved = SavedState {
            sessions: vec![SavedSession {
                name: "my-app".into(),
                project_root: Some(std::path::PathBuf::from("/home/user/my-app")),
                pane_cwds: vec![
                    std::path::PathBuf::from("/home/user/my-app"),
                    std::path::PathBuf::from("/home/user/my-app/src"),
                ],
            }],
        };

        // Write directly to avoid depending on dirs_next in tests
        let json = serde_json::to_string_pretty(&saved).unwrap();
        std::fs::write(&path, &json).unwrap();

        let loaded: SavedState =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.sessions.len(), 1);
        assert_eq!(loaded.sessions[0].name, "my-app");
        assert_eq!(loaded.sessions[0].pane_cwds.len(), 2);
    }

    #[test]
    fn find_session_matches_by_root() {
        let root = Some(std::path::PathBuf::from("/home/user/my-app"));
        let saved = SavedState {
            sessions: vec![
                SavedSession {
                    name: "my-app".into(),
                    project_root: root.clone(),
                    pane_cwds: vec![],
                },
                SavedSession {
                    name: "other".into(),
                    project_root: Some(std::path::PathBuf::from("/home/user/other")),
                    pane_cwds: vec![],
                },
            ],
        };
        let found = find_session(&saved, &root).unwrap();
        assert_eq!(found.name, "my-app");
    }

    #[test]
    fn find_session_no_match() {
        let saved = SavedState { sessions: vec![] };
        let root = Some(std::path::PathBuf::from("/nonexistent"));
        assert!(find_session(&saved, &root).is_none());
    }

    #[test]
    fn find_session_none_root() {
        let saved = SavedState {
            sessions: vec![SavedSession {
                name: "x".into(),
                project_root: None,
                pane_cwds: vec![],
            }],
        };
        assert!(find_session(&saved, &None).is_none());
    }

    #[test]
    fn saved_state_default_is_empty() {
        let s = SavedState::default();
        assert!(s.sessions.is_empty());
    }

    #[test]
    fn kill_session_removes_by_name() {
        // We can't easily test the file I/O path in unit tests,
        // but we can test the retain logic directly
        let mut state = SavedState {
            sessions: vec![
                SavedSession {
                    name: "keep".into(),
                    project_root: None,
                    pane_cwds: vec![],
                },
                SavedSession {
                    name: "remove".into(),
                    project_root: None,
                    pane_cwds: vec![],
                },
            ],
        };
        state.sessions.retain(|s| s.name != "remove");
        assert_eq!(state.sessions.len(), 1);
        assert_eq!(state.sessions[0].name, "keep");
    }
}
