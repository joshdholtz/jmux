use jmux_core::{AgentState, AppState, PaneState, Session};
use std::path::PathBuf;
use tokio::sync::mpsc;

use crate::app::App;
use crate::socket::SocketEvent;

fn make_pane(id: usize) -> PaneState {
    PaneState {
        id,
        name: format!("pane-{}", id),
        cwd: PathBuf::from("/tmp"),
        agent_state: AgentState::Idle,
        process_name: None,
        cmd: None,
    }
}

fn make_session(id: usize, pane_count: usize) -> Session {
    Session {
        id,
        project: None,
        panes: (0..pane_count).map(make_pane).collect(),
        active_pane: 0,
    }
}

fn make_app(sessions: Vec<Session>) -> App {
    let state = AppState {
        sessions,
        active_session: 0,
    };
    let (_tx, rx) = mpsc::channel::<SocketEvent>(1);
    App::new(state, rx, "/tmp/test.sock".to_string())
}

// --- apply_set_status ---

#[test]
fn apply_set_status_updates_active_pane() {
    let mut app = make_app(vec![make_session(0, 2)]);
    app.state.sessions[0].active_pane = 1;
    app.apply_set_status_pub(AgentState::Working {
        message: Some("hi".into()),
    });
    assert!(matches!(
        app.state.sessions[0].panes[1].agent_state,
        AgentState::Working { .. }
    ));
    // pane 0 unchanged
    assert_eq!(app.state.sessions[0].panes[0].agent_state, AgentState::Idle);
}

#[test]
fn apply_flash_sets_waiting() {
    let mut app = make_app(vec![make_session(0, 1)]);
    app.apply_flash_pub();
    assert!(matches!(
        app.state.sessions[0].panes[0].agent_state,
        AgentState::Waiting { .. }
    ));
}

// --- close_active_pane ---

#[test]
fn close_pane_removes_it() {
    let mut app = make_app(vec![make_session(0, 3)]);
    app.state.sessions[0].active_pane = 1;
    app.close_active_pane_pub();
    assert_eq!(app.state.sessions[0].panes.len(), 2);
    assert_eq!(app.state.sessions[0].panes[0].id, 0);
    assert_eq!(app.state.sessions[0].panes[1].id, 2);
}

#[test]
fn close_pane_does_not_close_last_pane() {
    let mut app = make_app(vec![make_session(0, 1)]);
    app.close_active_pane_pub();
    assert_eq!(app.state.sessions[0].panes.len(), 1);
}

#[test]
fn close_pane_clamps_active_index() {
    let mut app = make_app(vec![make_session(0, 2)]);
    app.state.sessions[0].active_pane = 1;
    app.close_active_pane_pub();
    assert_eq!(app.state.sessions[0].active_pane, 0);
}

// --- enter_copy_mode ---

#[test]
fn enter_copy_mode_sets_state() {
    let mut app = make_app(vec![make_session(0, 2)]);
    app.state.sessions[0].active_pane = 1;
    app.enter_copy_mode_pub();
    let cm = app.copy_mode.as_ref().unwrap();
    assert_eq!(cm.session_id, 0);
    assert_eq!(cm.pane_id, 1);
    assert_eq!(cm.scroll_offset, 0);
}

// --- session navigation ---

#[test]
fn active_session_wraps() {
    let mut app = make_app(vec![make_session(0, 1), make_session(1, 1)]);
    app.state.active_session = 1;
    // simulate next session wrapping to 0
    let len = app.state.sessions.len();
    app.state.active_session = (app.state.active_session + 1) % len;
    assert_eq!(app.state.active_session, 0);
}

// --- open_or_switch_session ---

#[test]
fn switch_to_existing_session_by_path() {
    use jmux_core::{ProjectInfo, ProjectKind};
    let root = PathBuf::from("/home/user/my-app");
    let mut session = make_session(0, 1);
    session.project = Some(ProjectInfo {
        name: "my-app".into(),
        root: root.clone(),
        kind: ProjectKind::Git,
    });
    let mut app = make_app(vec![make_session(1, 1), session]);
    app.state.active_session = 0;
    app.open_or_switch_session_pub(root);
    assert_eq!(app.state.active_session, 1);
}

#[test]
fn open_new_session_for_unknown_path() {
    let mut app = make_app(vec![make_session(0, 1)]);
    let path = PathBuf::from("/tmp/brand-new-project");
    app.open_or_switch_session_pub(path);
    assert_eq!(app.state.sessions.len(), 2);
    assert_eq!(app.state.active_session, 1);
}

// --- rendering tests ---

use ratatui::{backend::TestBackend, Terminal};

#[test]
fn render_wide_does_not_panic() {
    let app = make_app(vec![make_session(0, 2)]);
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| crate::layout::render_wide(f, &app))
        .unwrap();
    // If we get here without panic, rendering is functional
}

#[test]
fn render_narrow_does_not_panic() {
    let app = make_app(vec![make_session(0, 1)]);
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| crate::layout::render_narrow(f, &app))
        .unwrap();
}

#[test]
fn render_wide_with_working_agent_does_not_panic() {
    let mut app = make_app(vec![make_session(0, 2)]);
    app.state.sessions[0].panes[0].agent_state = AgentState::Working {
        message: Some("editing".into()),
    };
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| crate::layout::render_wide(f, &app))
        .unwrap();
}

#[test]
fn render_wide_zoomed_does_not_panic() {
    let mut app = make_app(vec![make_session(0, 3)]);
    app.zoomed = true;
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| crate::layout::render_wide(f, &app))
        .unwrap();
}

#[test]
fn dashboard_overlay_does_not_panic() {
    let mut app = make_app(vec![make_session(0, 2)]);
    app.show_dashboard = true;
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| crate::layout::render_wide(f, &app))
        .unwrap();
}
