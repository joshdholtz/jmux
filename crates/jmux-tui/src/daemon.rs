use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use jmux_core::{AgentState, AppState, PaneState, ProjectKind, Session};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::{broadcast, mpsc, Mutex};

use crate::pty::{detect_agent_state, parse_osc_state, PtyOutput, PtyPane};

struct Daemon {
    state: AppState,
    ptys: HashMap<(usize, usize), PtyPane>,
    broadcast: broadcast::Sender<String>,
    pty_tx: mpsc::Sender<PtyOutput>,
    /// Tracks what the user is currently typing per pane, so we can name the pane on Enter.
    input_bufs: HashMap<(usize, usize), String>,
}

pub async fn run_daemon(state: AppState, socket_path: &Path, rows: u16, cols: u16) -> Result<()> {
    let (broadcast_tx, _) = broadcast::channel::<String>(1024);
    let (pty_tx, mut pty_rx) = mpsc::channel::<PtyOutput>(256);

    let socket_str = socket_path.to_string_lossy().to_string();
    let mut ptys = HashMap::new();
    for session in &state.sessions {
        for pane in &session.panes {
            let cwd = if pane.cwd.exists() {
                pane.cwd.clone()
            } else {
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))
            };
            match PtyPane::spawn(
                session.id,
                pane.id,
                &cwd,
                rows,
                cols,
                &socket_str,
                pty_tx.clone(),
                pane.cmd.as_deref(),
            ) {
                Ok(pty) => {
                    ptys.insert((session.id, pane.id), pty);
                }
                Err(e) => {
                    eprintln!(
                        "jmux daemon: failed to spawn PTY for pane {}: {}",
                        pane.id, e
                    );
                }
            }
        }
    }

    let daemon = Arc::new(Mutex::new(Daemon {
        state,
        ptys,
        broadcast: broadcast_tx,
        pty_tx: pty_tx.clone(),
        input_bufs: HashMap::new(),
    }));

    let shell_bin_for_pty = {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        std::path::Path::new(&shell)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("shell")
            .to_string()
    };
    let daemon_pty = daemon.clone();
    tokio::spawn(async move {
        while let Some(out) = pty_rx.recv().await {
            let mut d = daemon_pty.lock().await;
            let (fg_running, fg_name_change) =
                if let Some(pty) = d.ptys.get_mut(&(out.session_id, out.pane_id)) {
                    pty.process_output(&out.data);
                    pty.poll_fg_process()
                } else {
                    (false, None)
                };

            // Update pane name from foreground process on every output chunk.
            // poll_fg_process only calls `ps` when the PID changes, so this is cheap.
            let mut state_dirty = false;
            if let Some(new_name) = fg_name_change {
                if let Some(sess) = d.state.sessions.iter_mut().find(|s| s.id == out.session_id) {
                    if let Some(pane) = sess.panes.iter_mut().find(|p| p.id == out.pane_id) {
                        // Only update name when a real process is in the foreground.
                        // Don't clear when fg returns to shell — keep showing the last command.
                        if !new_name.is_empty() && pane.process_name.as_deref() != Some(&new_name) {
                            pane.process_name = Some(new_name);
                            state_dirty = true;
                        }
                        if !fg_running
                            && matches!(
                                pane.agent_state,
                                AgentState::Working { .. } | AgentState::Waiting { .. }
                            )
                        {
                            pane.agent_state = AgentState::Idle;
                            state_dirty = true;
                        }
                    }
                }
            }

            let new_state = parse_osc_state(&out.data).or_else(|| detect_agent_state(&out.data));
            if let Some(new_state) = new_state {
                update_agent_state(&mut d.state, out.session_id, out.pane_id, new_state);
                state_dirty = true;
            }

            // Broadcast raw PTY bytes so attached clients can render live output
            let pty_msg = serde_json::json!({
                "type": "pty",
                "session_id": out.session_id,
                "pane_id": out.pane_id,
                "data": out.data,
            })
            .to_string();
            let _ = d.broadcast.send(pty_msg);
            // Only broadcast state when something actually changed
            if state_dirty {
                let sm = state_msg(&d.state);
                let _ = d.broadcast.send(sm);
            }
        }
    });
    let _ = shell_bin_for_pty; // used by poll task below

    // Poll foreground process every 500ms — updates pane names and clears stale agent states.
    let daemon_poll = daemon.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
        loop {
            interval.tick().await;
            let mut d = daemon_poll.lock().await;
            let mut changed = false;
            // Collect updates outside borrow to avoid simultaneous mut/shared refs
            let keys: Vec<(usize, usize)> = d.ptys.keys().copied().collect();
            for (sid, pid) in keys {
                let (running, name_change) = if let Some(pty) = d.ptys.get_mut(&(sid, pid)) {
                    pty.poll_fg_process()
                } else {
                    continue;
                };
                // Update process name in AppState
                if let Some(new_name) = name_change {
                    if let Some(sess) = d.state.sessions.iter_mut().find(|s| s.id == sid) {
                        if let Some(pane) = sess.panes.iter_mut().find(|p| p.id == pid) {
                            // Only update when a real process is fg — don't clear on return to shell
                            if !new_name.is_empty()
                                && pane.process_name.as_deref() != Some(&new_name)
                            {
                                pane.process_name = Some(new_name);
                                changed = true;
                            }
                            if !running
                                && matches!(
                                    pane.agent_state,
                                    AgentState::Working { .. } | AgentState::Waiting { .. }
                                )
                            {
                                pane.agent_state = AgentState::Idle;
                                changed = true;
                            }
                        }
                    }
                } else if !running {
                    // No name change but process finished — still clear state
                    if let Some(sess) = d.state.sessions.iter_mut().find(|s| s.id == sid) {
                        if let Some(pane) = sess.panes.iter_mut().find(|p| p.id == pid) {
                            if matches!(
                                pane.agent_state,
                                AgentState::Working { .. } | AgentState::Waiting { .. }
                            ) {
                                pane.agent_state = AgentState::Idle;
                                changed = true;
                            }
                        }
                    }
                }
            }
            if changed {
                let sm = state_msg(&d.state);
                let _ = d.broadcast.send(sm);
            }
        }
    });

    if socket_path.exists() {
        std::fs::remove_file(socket_path)?;
    }
    let listener = UnixListener::bind(socket_path)?;
    eprintln!("jmux daemon listening on {}", socket_path.display());

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let d = daemon.clone();
                tokio::spawn(handle_conn(stream, d));
            }
            Err(e) => eprintln!("jmux daemon accept error: {}", e),
        }
    }
}

fn to_saved_state(state: &AppState) -> jmux_core::persistence::SavedState {
    use jmux_core::persistence::{SavedSession, SavedState};
    SavedState {
        sessions: state
            .sessions
            .iter()
            .map(|s| SavedSession {
                name: s
                    .project
                    .as_ref()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| format!("session-{}", s.id)),
                project_root: s.project.as_ref().map(|p| p.root.clone()),
                pane_cwds: s.panes.iter().map(|p| p.cwd.clone()).collect(),
            })
            .collect(),
    }
}

fn state_msg(state: &AppState) -> String {
    serde_json::json!({"type": "state", "data": state}).to_string()
}

fn update_agent_state(state: &mut AppState, session_id: usize, pane_id: usize, new: AgentState) {
    if let Some(s) = state.sessions.iter_mut().find(|s| s.id == session_id) {
        if let Some(p) = s.panes.iter_mut().find(|p| p.id == pane_id) {
            p.agent_state = new;
        }
    }
}

async fn maybe_recv(rx: &mut Option<broadcast::Receiver<String>>) -> Option<String> {
    if let Some(r) = rx.as_mut() {
        r.recv().await.ok()
    } else {
        std::future::pending::<Option<String>>().await
    }
}

async fn handle_conn(stream: tokio::net::UnixStream, daemon: Arc<Mutex<Daemon>>) {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let mut broadcast_rx: Option<broadcast::Receiver<String>> = None;

    loop {
        tokio::select! {
            line = lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        if let Err(e) =
                            handle_request(&line, &daemon, &mut broadcast_rx, &mut writer).await
                        {
                            eprintln!("jmux daemon request error: {}", e);
                        }
                    }
                    _ => break,
                }
            }
            Some(msg) = maybe_recv(&mut broadcast_rx) => {
                if writer.write_all(msg.as_bytes()).await.is_err() {
                    break;
                }
                if writer.write_all(b"\n").await.is_err() {
                    break;
                }
            }
        }
    }
}

async fn handle_request(
    line: &str,
    daemon: &Arc<Mutex<Daemon>>,
    broadcast_rx: &mut Option<broadcast::Receiver<String>>,
    writer: &mut (impl AsyncWriteExt + Unpin),
) -> Result<()> {
    let req: serde_json::Value = serde_json::from_str(line)?;
    let method = req["method"].as_str().unwrap_or("");
    let params = &req["params"];

    match method {
        "get-state" => {
            let d = daemon.lock().await;
            let msg = state_msg(&d.state);
            drop(d);
            writer.write_all(msg.as_bytes()).await?;
            writer.write_all(b"\n").await?;
        }
        "subscribe" => {
            let d = daemon.lock().await;
            let initial = state_msg(&d.state);
            *broadcast_rx = Some(d.broadcast.subscribe());
            drop(d);
            writer.write_all(initial.as_bytes()).await?;
            writer.write_all(b"\n").await?;
        }
        "input" => {
            let session_id = params["session_id"].as_u64().unwrap_or(0) as usize;
            let pane_id = params["pane_id"].as_u64().unwrap_or(0) as usize;
            let data: Vec<u8> = params["data"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_u64().map(|b| b as u8))
                        .collect()
                })
                .unwrap_or_default();
            let mut d = daemon.lock().await;
            if let Some(pty) = d.ptys.get_mut(&(session_id, pane_id)) {
                let _ = pty.write_input(&data);
            }

            // Only track typed input when the shell is in the foreground.
            // If vim/htop/etc. is running, Enter means something else entirely.
            let shell_is_fg = d
                .ptys
                .get(&(session_id, pane_id))
                .map(|pty| {
                    let fg = pty.master.process_group_leader().map(|p| p as u32);
                    fg == pty.shell_pid
                })
                .unwrap_or(false);

            if shell_is_fg {
                let mut entered_cmd: Option<String> = None;
                {
                    let buf = d.input_bufs.entry((session_id, pane_id)).or_default();
                    for &byte in &data {
                        match byte {
                            b'\r' | b'\n' => {
                                entered_cmd = buf.split_whitespace().next().map(|s| s.to_string());
                                buf.clear();
                            }
                            0x7f | 0x08 => {
                                buf.pop();
                            } // backspace
                            0x15 => {
                                buf.clear();
                            } // ctrl-u
                            0x01..=0x1f => {
                                buf.clear();
                            } // other control chars — bail
                            b if b.is_ascii() => {
                                buf.push(b as char);
                            }
                            _ => {}
                        }
                    }
                }
                if let Some(cmd) = entered_cmd {
                    if let Some(sess) = d.state.sessions.iter_mut().find(|s| s.id == session_id) {
                        if let Some(pane) = sess.panes.iter_mut().find(|p| p.id == pane_id) {
                            pane.process_name = Some(cmd);
                        }
                    }
                    let sm = state_msg(&d.state);
                    let _ = d.broadcast.send(sm);
                }
            }

            // User typed — clear Waiting so the pane stops flashing
            if let Some(sess) = d.state.sessions.iter_mut().find(|s| s.id == session_id) {
                if let Some(pane) = sess.panes.iter_mut().find(|p| p.id == pane_id) {
                    if matches!(pane.agent_state, AgentState::Waiting { .. }) {
                        pane.agent_state = AgentState::Idle;
                        let sm = state_msg(&d.state);
                        let _ = d.broadcast.send(sm);
                    }
                }
            }
        }
        "resize" => {
            // Global resize — only used for initial connect; per-pane takes precedence
            let rows = params["rows"].as_u64().unwrap_or(24) as u16;
            let cols = params["cols"].as_u64().unwrap_or(80) as u16;
            let mut d = daemon.lock().await;
            for pty in d.ptys.values_mut() {
                if pty.rows != rows || pty.cols != cols {
                    let _ = pty.resize(rows, cols);
                }
            }
        }
        "resize-pane" => {
            let session_id = params["session_id"].as_u64().unwrap_or(0) as usize;
            let pane_id = params["pane_id"].as_u64().unwrap_or(0) as usize;
            let rows = params["rows"].as_u64().unwrap_or(24) as u16;
            let cols = params["cols"].as_u64().unwrap_or(80) as u16;
            let mut d = daemon.lock().await;
            if let Some(pty) = d.ptys.get_mut(&(session_id, pane_id)) {
                if pty.rows != rows || pty.cols != cols {
                    let _ = pty.resize(rows, cols);
                }
            }
        }
        "set-status" => {
            let state_str = params["state"].as_str().unwrap_or("idle");
            let msg = params["message"].as_str().map(|s| s.to_string());
            let agent_state = match state_str {
                "working" => AgentState::Working { message: msg },
                "waiting" => AgentState::Waiting { message: msg },
                "error" => AgentState::Error { message: msg },
                _ => AgentState::Idle,
            };
            let session_id = params["session_id"].as_u64().map(|v| v as usize);
            let pane_id = params["pane_id"].as_u64().map(|v| v as usize);
            let mut d = daemon.lock().await;
            let target_pane = if let (Some(sid), Some(pid)) = (session_id, pane_id) {
                d.state
                    .sessions
                    .iter_mut()
                    .find(|s| s.id == sid)
                    .and_then(|s| s.panes.iter_mut().find(|p| p.id == pid))
            } else {
                let idx = d.state.active_session;
                d.state.sessions.get_mut(idx).and_then(|s| {
                    let pi = s.active_pane;
                    s.panes.get_mut(pi)
                })
            };
            if let Some(pane) = target_pane {
                pane.agent_state = agent_state;
            }
            let sm = state_msg(&d.state);
            let _ = d.broadcast.send(sm);
        }
        "flash" => {
            let session_id = params["session_id"].as_u64().map(|v| v as usize);
            let pane_id = params["pane_id"].as_u64().map(|v| v as usize);
            let mut d = daemon.lock().await;
            let flash_state = AgentState::Waiting {
                message: Some("Flash!".to_string()),
            };
            let updated = if let (Some(sid), Some(pid)) = (session_id, pane_id) {
                if let Some(session) = d.state.sessions.iter_mut().find(|s| s.id == sid) {
                    if let Some(pane) = session.panes.iter_mut().find(|p| p.id == pid) {
                        pane.agent_state = flash_state;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };
            if !updated {
                let idx = d.state.active_session;
                if let Some(session) = d.state.sessions.get_mut(idx) {
                    let pane_idx = session.active_pane;
                    if let Some(pane) = session.panes.get_mut(pane_idx) {
                        pane.agent_state = AgentState::Waiting {
                            message: Some("Flash!".to_string()),
                        };
                    }
                }
            }
            let sm = state_msg(&d.state);
            let _ = d.broadcast.send(sm);
        }
        "new-session" => {
            if let Some(path_str) = params["path"].as_str() {
                let path = PathBuf::from(path_str);
                let mut d = daemon.lock().await;
                let exists = d
                    .state
                    .sessions
                    .iter()
                    .any(|s| s.project.as_ref().map(|p| &p.root) == Some(&path));
                if !exists {
                    let new_id = d.state.sessions.iter().map(|s| s.id).max().unwrap_or(0) + 1;
                    let project = jmux_core::detect_project(&path).or_else(|| {
                        let name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("project")
                            .to_string();
                        Some(jmux_core::ProjectInfo {
                            name,
                            root: path.clone(),
                            kind: ProjectKind::Plain,
                        })
                    });
                    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
                    let shell_bin = std::path::Path::new(&shell)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("shell")
                        .to_string();
                    let pane = PaneState {
                        id: 0,
                        name: shell_bin,
                        cwd: path.clone(),
                        agent_state: AgentState::Idle,
                        process_name: None,
                        cmd: None,
                    };
                    d.state.sessions.push(Session {
                        id: new_id,
                        project,
                        panes: vec![pane],
                        active_pane: 0,
                    });
                    d.state.active_session = d.state.sessions.len() - 1;
                    // Spawn a PTY for the new pane
                    let (rows, cols) = d
                        .ptys
                        .values()
                        .next()
                        .map(|p| (p.rows, p.cols))
                        .unwrap_or((24, 200));
                    let socket = std::env::var("JMUX_SOCKET").unwrap_or_default();
                    let pty_tx = d.pty_tx.clone();
                    if let Ok(pty) =
                        PtyPane::spawn(new_id, 0, &path, rows, cols, &socket, pty_tx, None)
                    {
                        d.ptys.insert((new_id, 0), pty);
                    }
                    let sm = state_msg(&d.state);
                    let _ = d.broadcast.send(sm);
                }
            }
        }
        "set-cwd" => {
            if let Some(path_str) = params["path"].as_str() {
                let path = PathBuf::from(path_str);
                let mut d = daemon.lock().await;
                let idx = d.state.active_session;
                if let Some(session) = d.state.sessions.get_mut(idx) {
                    let pane_idx = session.active_pane;
                    if let Some(pane) = session.panes.get_mut(pane_idx) {
                        pane.cwd = path;
                    }
                }
            }
        }
        "set-name" => {
            let name = params["name"].as_str().unwrap_or("").trim().to_string();
            let session_id = params["session_id"].as_u64().map(|v| v as usize);
            let pane_id = params["pane_id"].as_u64().map(|v| v as usize);
            let mut d = daemon.lock().await;
            let target = if let (Some(sid), Some(pid)) = (session_id, pane_id) {
                d.state
                    .sessions
                    .iter_mut()
                    .find(|s| s.id == sid)
                    .and_then(|s| s.panes.iter_mut().find(|p| p.id == pid))
            } else {
                let idx = d.state.active_session;
                d.state.sessions.get_mut(idx).and_then(|s| {
                    let pi = s.active_pane;
                    s.panes.get_mut(pi)
                })
            };
            if let Some(pane) = target {
                pane.process_name = if name.is_empty() { None } else { Some(name) };
                let sm = state_msg(&d.state);
                let _ = d.broadcast.send(sm);
            }
        }
        "close-pane" => {
            let session_id = params["session_id"].as_u64().unwrap_or(0) as usize;
            let pane_id = params["pane_id"].as_u64().unwrap_or(0) as usize;
            let mut d = daemon.lock().await;
            d.ptys.remove(&(session_id, pane_id));
            if let Some(sess) = d.state.sessions.iter_mut().find(|s| s.id == session_id) {
                if let Some(idx) = sess.panes.iter().position(|p| p.id == pane_id) {
                    sess.panes.remove(idx);
                    if sess.active_pane >= sess.panes.len() && !sess.panes.is_empty() {
                        sess.active_pane = sess.panes.len() - 1;
                    }
                }
            }
            d.state.sessions.retain(|s| !s.panes.is_empty());
            if d.state.active_session >= d.state.sessions.len() && !d.state.sessions.is_empty() {
                d.state.active_session = d.state.sessions.len() - 1;
            }
            let sm = state_msg(&d.state);
            let _ = d.broadcast.send(sm);
        }
        "add-pane" => {
            let session_id = params["session_id"].as_u64().unwrap_or(0) as usize;
            let cwd = params["cwd"]
                .as_str()
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")));
            let mut d = daemon.lock().await;
            if let Some(session) = d.state.sessions.iter_mut().find(|s| s.id == session_id) {
                let new_id = session.panes.iter().map(|p| p.id).max().unwrap_or(0) + 1;
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
                let shell_bin = std::path::Path::new(&shell)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("shell")
                    .to_string();
                session.panes.push(PaneState {
                    id: new_id,
                    name: shell_bin,
                    cwd: cwd.clone(),
                    agent_state: AgentState::Idle,
                    process_name: None,
                    cmd: None,
                });
                session.active_pane = session.panes.len() - 1;
                let (rows, cols) = d
                    .ptys
                    .values()
                    .next()
                    .map(|p| (p.rows, p.cols))
                    .unwrap_or((24, 200));
                let socket = std::env::var("JMUX_SOCKET").unwrap_or_default();
                let pty_tx = d.pty_tx.clone();
                if let Ok(pty) =
                    PtyPane::spawn(session_id, new_id, &cwd, rows, cols, &socket, pty_tx, None)
                {
                    d.ptys.insert((session_id, new_id), pty);
                }
                let sm = state_msg(&d.state);
                let _ = d.broadcast.send(sm);
            }
        }
        "kill-session" => {
            let name = params["name"].as_str().unwrap_or("");
            let mut d = daemon.lock().await;
            if let Some(sess_idx) = d.state.sessions.iter().position(|s| {
                s.project.as_ref().map(|p| p.name.as_str()).unwrap_or("") == name
                    || format!("session-{}", s.id) == name
            }) {
                let session = &d.state.sessions[sess_idx];
                let pane_keys: Vec<(usize, usize)> =
                    session.panes.iter().map(|p| (session.id, p.id)).collect();
                for key in pane_keys {
                    d.ptys.remove(&key);
                }
                d.state.sessions.remove(sess_idx);
                if d.state.active_session >= d.state.sessions.len() && !d.state.sessions.is_empty()
                {
                    d.state.active_session = d.state.sessions.len() - 1;
                }
                let sm = state_msg(&d.state);
                let _ = d.broadcast.send(sm);
            }
        }
        "focus-pane" => {
            let session_id = params["session_id"].as_u64().unwrap_or(0) as usize;
            let pane_id = params["pane_id"].as_u64().unwrap_or(0) as usize;
            let mut d = daemon.lock().await;
            if let Some(sess) = d.state.sessions.iter_mut().find(|s| s.id == session_id) {
                if let Some(pane) = sess.panes.iter_mut().find(|p| p.id == pane_id) {
                    if matches!(pane.agent_state, AgentState::Waiting { .. }) {
                        pane.agent_state = AgentState::Idle;
                        let sm = state_msg(&d.state);
                        let _ = d.broadcast.send(sm);
                    }
                }
            }
        }
        "detach" => {
            // Persist current state so `jmux ls` and restart can see it
            let d = daemon.lock().await;
            let saved = to_saved_state(&d.state);
            let _ = jmux_core::persistence::save(&saved);
        }
        other => {
            eprintln!("jmux daemon: unknown method '{}'", other);
        }
    }
    Ok(())
}
