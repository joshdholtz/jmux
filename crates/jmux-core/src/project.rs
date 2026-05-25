use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProjectKind {
    Git,
    Cargo,
    Node,
    Go,
    Python,
    Plain,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub name: String,
    pub root: PathBuf,
    pub kind: ProjectKind,
}

pub fn detect_project(cwd: &Path) -> Option<ProjectInfo> {
    let mut current = cwd.to_path_buf();
    loop {
        if let Some(info) = check_dir(&current) {
            return Some(info);
        }
        if !current.pop() {
            break;
        }
    }
    None
}

fn check_dir(dir: &Path) -> Option<ProjectInfo> {
    let markers: &[(&str, ProjectKind)] = &[
        (".jmux.toml", ProjectKind::Plain),
        (".git", ProjectKind::Git),
        ("Cargo.toml", ProjectKind::Cargo),
        ("package.json", ProjectKind::Node),
        ("go.mod", ProjectKind::Go),
        ("pyproject.toml", ProjectKind::Python),
    ];

    for (marker, kind) in markers {
        if dir.join(marker).exists() {
            let name = project_name_from_path(dir);
            return Some(ProjectInfo {
                name,
                root: dir.to_path_buf(),
                kind: kind.clone(),
            });
        }
    }
    None
}

pub fn project_name_from_path(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn detects_git_project() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        let info = detect_project(dir.path()).unwrap();
        assert_eq!(info.kind, ProjectKind::Git);
        assert_eq!(info.root, dir.path());
    }

    #[test]
    fn detects_cargo_project() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let info = detect_project(dir.path()).unwrap();
        assert_eq!(info.kind, ProjectKind::Cargo);
    }

    #[test]
    fn detects_node_project() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("package.json"), "{}").unwrap();
        let info = detect_project(dir.path()).unwrap();
        assert_eq!(info.kind, ProjectKind::Node);
    }

    #[test]
    fn detects_go_project() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("go.mod"), "module example").unwrap();
        let info = detect_project(dir.path()).unwrap();
        assert_eq!(info.kind, ProjectKind::Go);
    }

    #[test]
    fn detects_python_project() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("pyproject.toml"), "[tool]").unwrap();
        let info = detect_project(dir.path()).unwrap();
        assert_eq!(info.kind, ProjectKind::Python);
    }

    #[test]
    fn returns_none_for_empty_dir() {
        let dir = tempdir().unwrap();
        assert!(detect_project(dir.path()).is_none());
    }

    #[test]
    fn walks_up_to_parent() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        let subdir = dir.path().join("src").join("auth");
        fs::create_dir_all(&subdir).unwrap();
        let info = detect_project(&subdir).unwrap();
        assert_eq!(info.root, dir.path());
    }

    #[test]
    fn jmux_toml_takes_priority_over_git() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join(".jmux.toml"), "").unwrap();
        let info = detect_project(dir.path()).unwrap();
        assert_eq!(info.kind, ProjectKind::Plain);
    }

    #[test]
    fn project_name_from_path_extracts_last_segment() {
        let path = std::path::Path::new("/home/user/dev/my-app");
        assert_eq!(project_name_from_path(path), "my-app");
    }

    #[test]
    fn project_name_from_root() {
        let path = std::path::Path::new("/");
        // Should not panic
        let _ = project_name_from_path(path);
    }
}
