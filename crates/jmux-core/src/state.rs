use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::project::ProjectInfo;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AgentState {
    Idle,
    Working { message: Option<String> },
    Waiting { message: Option<String> },
    Error { message: Option<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneState {
    pub id: usize,
    pub name: String,
    pub cwd: PathBuf,
    pub agent_state: AgentState,
    pub process_name: Option<String>,
    #[serde(default)]
    pub cmd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: usize,
    pub project: Option<ProjectInfo>,
    pub panes: Vec<PaneState>,
    pub active_pane: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingSelect {
    pub items: Vec<String>,
    pub selected: usize,
    pub session_id: usize,
    pub pane_id: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppState {
    pub sessions: Vec<Session>,
    pub active_session: usize,
    #[serde(default)]
    pub pending_select: Option<PendingSelect>,
}

impl AppState {
    pub fn attention_count(&self) -> usize {
        self.sessions
            .iter()
            .flat_map(|s| s.panes.iter())
            .filter(|p| {
                matches!(
                    p.agent_state,
                    AgentState::Waiting { .. } | AgentState::Error { .. }
                )
            })
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_pane(id: usize, state: AgentState) -> PaneState {
        PaneState {
            id,
            name: format!("pane-{}", id),
            cwd: PathBuf::from("/tmp"),
            agent_state: state,
            process_name: None,
            cmd: None,
        }
    }

    fn make_session(id: usize, panes: Vec<PaneState>) -> Session {
        Session {
            id,
            project: None,
            panes,
            active_pane: 0,
        }
    }

    #[test]
    fn attention_count_empty() {
        let state = AppState {
            sessions: vec![],
            active_session: 0,
            pending_select: None,
        };
        assert_eq!(state.attention_count(), 0);
    }

    #[test]
    fn attention_count_only_idle() {
        let state = AppState {
            sessions: vec![make_session(
                0,
                vec![
                    make_pane(0, AgentState::Idle),
                    make_pane(1, AgentState::Working { message: None }),
                ],
            )],
            active_session: 0,
            pending_select: None,
        };
        assert_eq!(state.attention_count(), 0);
    }

    #[test]
    fn attention_count_waiting_and_error() {
        let state = AppState {
            sessions: vec![
                make_session(
                    0,
                    vec![
                        make_pane(0, AgentState::Waiting { message: None }),
                        make_pane(1, AgentState::Idle),
                    ],
                ),
                make_session(
                    1,
                    vec![make_pane(
                        0,
                        AgentState::Error {
                            message: Some("oops".into()),
                        },
                    )],
                ),
            ],
            active_session: 0,
            pending_select: None,
        };
        assert_eq!(state.attention_count(), 2);
    }

    #[test]
    fn agent_state_eq() {
        assert_eq!(AgentState::Idle, AgentState::Idle);
        assert_ne!(AgentState::Idle, AgentState::Working { message: None });
        assert_eq!(
            AgentState::Waiting {
                message: Some("x".into())
            },
            AgentState::Waiting {
                message: Some("x".into())
            },
        );
    }
}
