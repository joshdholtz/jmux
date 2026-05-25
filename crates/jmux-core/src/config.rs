use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct JmuxConfig {
    pub project: Option<ProjectConfig>,
    #[serde(default)]
    pub panes: Vec<PaneConfig>,
}

#[derive(Debug, Deserialize)]
pub struct ProjectConfig {
    pub name: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct PaneConfig {
    pub name: String,
    pub cmd: Option<String>,
}

pub fn load_config(project_root: &std::path::Path) -> Option<JmuxConfig> {
    let path = project_root.join(".jmux.toml");
    let text = std::fs::read_to_string(path).ok()?;
    toml::from_str(&text).ok()
}
