use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Stdout;
use std::path::PathBuf;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use jmux_core::{AgentState, AppState, PaneState};
use ratatui::{backend::CrosstermBackend, layout::Rect, Terminal};
use tokio::sync::mpsc;

use crate::daemon_client::DaemonClient;
use crate::input::key_to_bytes;
use crate::layout::{self, LayoutMode};
use crate::pty::{PtyOutput, PtyPane};
use crate::socket::SocketEvent;

pub struct CopyMode {
    pub session_id: usize,
    pub pane_id: usize,
    pub scroll_offset: usize, // lines from the bottom (0 = at bottom/live)
}

/// What the rename prompt is currently editing.
#[derive(Debug, Clone, PartialEq)]
pub enum RenameTarget {
    Pane,
    Session,
}

pub struct App {
    pub state: AppState,
    pub layout_mode: LayoutMode,
    pub show_dashboard: bool,
    pub zoomed: bool,
    pub git_branch: Option<String>,
    pub prefix_mode: bool,
    pub last_size: (u16, u16),
    pub show_picker: bool,
    pub picker: Option<crate::widgets::picker::PickerState>,
    pub copy_mode: Option<CopyMode>,
    pub rename_input: Option<String>,
    /// Whether the rename prompt is editing a pane or a session name.
    pub rename_target: RenameTarget,
    /// When Some, TUI is connected to a background daemon instead of owning PTYs directly.
    pub daemon_client: Option<DaemonClient>,
    quit_requested: bool,
    detach_requested: bool,
    socket_rx: mpsc::Receiver<SocketEvent>,
    pty_rx: mpsc::Receiver<PtyOutput>,
    pty_tx: mpsc::Sender<PtyOutput>,
    pub ptys: HashMap<(usize, usize), PtyPane>,
    /// Fake PTY panes in daemon mode — parsers fed from daemon PTY byte stream.
    pub client_ptys: HashMap<(usize, usize), PtyPane>,
    pub socket_path: String,
    pub pane_rects: RefCell<HashMap<(usize, usize), Rect>>,
    pub session_rects: RefCell<Vec<Rect>>,
    /// Click targets for pane rows in the sidebar: (sess_id, pane_id) → Rect
    pub sidebar_pane_rects: RefCell<HashMap<(usize, usize), Rect>>,
}

impl App {
    pub fn new(
        state: AppState,
        socket_rx: mpsc::Receiver<SocketEvent>,
        socket_path: String,
    ) -> Self {
        let (pty_tx, pty_rx) = mpsc::channel::<PtyOutput>(256);
        Self {
            state,
            layout_mode: LayoutMode::Wide,
            show_dashboard: false,
            zoomed: false,
            git_branch: None,
            prefix_mode: false,
            last_size: (22, 200),
            show_picker: false,
            picker: None,
            copy_mode: None,
            rename_input: None,
            rename_target: RenameTarget::Pane,
            daemon_client: None,
            quit_requested: false,
            detach_requested: false,
            socket_rx,
            pty_rx,
            pty_tx,
            ptys: HashMap::new(),
            client_ptys: HashMap::new(),
            socket_path,
            pane_rects: RefCell::new(HashMap::new()),
            session_rects: RefCell::new(Vec::new()),
            sidebar_pane_rects: RefCell::new(HashMap::new()),
        }
    }

    pub fn new_daemon_client(
        state: AppState,
        client: DaemonClient,
        socket_rx: mpsc::Receiver<SocketEvent>,
        socket_path: String,
    ) -> Self {
        let (pty_tx, pty_rx) = mpsc::channel::<PtyOutput>(256);
        // Pre-populate client panes from state (will be fed bytes from daemon)
        let mut client_ptys = HashMap::new();
        for session in &state.sessions {
            for pane in &session.panes {
                client_ptys.insert((session.id, pane.id), PtyPane::new_client(22, 200));
            }
        }
        Self {
            state,
            layout_mode: LayoutMode::Wide,
            show_dashboard: false,
            zoomed: false,
            git_branch: None,
            prefix_mode: false,
            last_size: (22, 200),
            show_picker: false,
            picker: None,
            copy_mode: None,
            rename_input: None,
            rename_target: RenameTarget::Pane,
            daemon_client: Some(client),
            quit_requested: false,
            detach_requested: false,
            socket_rx,
            pty_rx,
            pty_tx,
            ptys: HashMap::new(),
            client_ptys,
            socket_path,
            pane_rects: RefCell::new(HashMap::new()),
            session_rects: RefCell::new(Vec::new()),
            sidebar_pane_rects: RefCell::new(HashMap::new()),
        }
    }

    pub fn is_daemon_mode(&self) -> bool {
        self.daemon_client.is_some()
    }

    /// Returns the PTY (local or client) for the given pane, if available.
    pub fn pty_for(&self, sess_id: usize, pane_id: usize) -> Option<&PtyPane> {
        self.ptys
            .get(&(sess_id, pane_id))
            .or_else(|| self.client_ptys.get(&(sess_id, pane_id)))
    }

    fn spawn_all_ptys(&mut self, rows: u16, cols: u16) -> Result<()> {
        if self.daemon_client.is_some() {
            return Ok(());
        }
        for session in &self.state.sessions {
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
                    &self.socket_path,
                    self.pty_tx.clone(),
                    pane.cmd.as_deref(),
                ) {
                    Ok(pty) => {
                        self.ptys.insert((session.id, pane.id), pty);
                    }
                    Err(e) => {
                        eprintln!(
                            "jmux: failed to spawn PTY for session {} pane {}: {}",
                            session.id, pane.id, e
                        );
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn run(
        mut self,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> Result<AppState> {
        let size = terminal.size()?;
        let rows = size.height.saturating_sub(2);
        let cols = size.width;
        self.last_size = (rows, cols);
        self.spawn_all_ptys(rows, cols)?;

        // Subscribe to daemon state updates if in daemon mode
        if let Some(ref client) = self.daemon_client {
            let _ = client.subscribe().await;
            let _ = client.send_resize(rows, cols).await;
        }

        // One-time git branch lookup
        self.git_branch = {
            let root = self
                .state
                .sessions
                .get(self.state.active_session)
                .and_then(|s| s.project.as_ref())
                .map(|p| p.root.clone());
            root.and_then(|r| {
                std::process::Command::new("git")
                    .args([
                        "-C",
                        r.to_string_lossy().as_ref(),
                        "rev-parse",
                        "--abbrev-ref",
                        "HEAD",
                    ])
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            })
        };

        // Daemon state watch channel (used when in daemon mode)
        let mut daemon_state_rx = self.daemon_client.as_ref().map(|c| c.state_rx());
        // Daemon PTY byte stream (used when in daemon mode)
        let daemon_pty_rx = self.daemon_client.as_ref().map(|c| c.pty_rx.clone());
        // Process-based agent state polling
        let mut process_poll = tokio::time::interval(std::time::Duration::from_millis(500));

        loop {
            if self.quit_requested || self.detach_requested {
                break;
            }

            let width = terminal.size()?.width;
            self.layout_mode = layout::detect_layout(width);

            terminal.draw(|f| self.render(f))?;
            self.sync_pty_sizes();

            tokio::select! {
                // Frame timer ~60fps
                _ = tokio::time::sleep(std::time::Duration::from_millis(16)) => {
                    // Fall through to poll terminal events below
                }
                // Process-based agent state detection (every 500ms)
                _ = process_poll.tick() => {
                    for session in &mut self.state.sessions {
                        for pane in &mut session.panes {
                            if let Some(pty) = self.ptys.get(&(session.id, pane.id)) {
                                let running = pty.is_process_running();
                                let fg_pid = pty.master.process_group_leader().map(|p| p as u32);

                                // Update pane name from foreground process
                                if let Some(pid) = fg_pid {
                                    if Some(pid) != pty.shell_pid {
                                        if let Some(name) = process_name(pid) {
                                            pane.name = name;
                                        }
                                    } else {
                                        // Back to shell — reset to shell binary name
                                        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
                                        let bin = std::path::Path::new(&shell)
                                            .file_name().and_then(|n| n.to_str()).unwrap_or("shell").to_string();
                                        pane.name = bin;
                                    }
                                }

                                match &pane.agent_state {
                                    AgentState::Idle if running => {
                                        pane.agent_state = AgentState::Working { message: None };
                                    }
                                    AgentState::Working { .. } | AgentState::Waiting { .. }
                                        if !running =>
                                    {
                                        pane.agent_state = AgentState::Idle;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    continue;
                }
                // Daemon state updates (only active in daemon mode)
                _ = async {
                    if let Some(ref mut rx) = daemon_state_rx {
                        let _ = rx.changed().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    if let Some(ref rx) = daemon_state_rx {
                        if let Some(new_state) = rx.borrow().clone() {
                            // Preserve local active_session/active_pane selections
                            let active_session = self.state.active_session;
                            let active_panes: Vec<usize> = self.state.sessions.iter()
                                .map(|s| s.active_pane)
                                .collect();
                            self.state = new_state;
                            // Restore local selections if still valid
                            if active_session < self.state.sessions.len() {
                                self.state.active_session = active_session;
                            }
                            for (i, &ap) in active_panes.iter().enumerate() {
                                if let Some(session) = self.state.sessions.get_mut(i) {
                                    if ap < session.panes.len() {
                                        session.active_pane = ap;
                                    }
                                }
                            }
                            // Ensure client_ptys exists for every pane in new state
                            let (rows, cols) = self.last_size;
                            for session in &self.state.sessions {
                                for pane in &session.panes {
                                    self.client_ptys
                                        .entry((session.id, pane.id))
                                        .or_insert_with(|| PtyPane::new_client(rows, cols));
                                }
                            }
                        }
                    }
                    continue;
                }
                // Daemon PTY byte stream — feed into client pane parsers
                maybe_pty_msg = async {
                    if let Some(ref rx_arc) = daemon_pty_rx {
                        rx_arc.lock().await.recv().await
                    } else {
                        std::future::pending().await
                    }
                } => {
                    if let Some(msg) = maybe_pty_msg {
                        // Ensure client pane exists (new panes added while attached)
                        let key = (msg.session_id, msg.pane_id);
                        let (rows, cols) = self.last_size;
                        self.client_ptys.entry(key).or_insert_with(|| PtyPane::new_client(rows, cols));
                        if let Some(pty) = self.client_ptys.get_mut(&key) {
                            pty.process_output(&msg.data);
                        }
                    }
                    continue;
                }
                // Socket events
                maybe_event = self.socket_rx.recv() => {
                    match maybe_event {
                        Some(SocketEvent::SetStatus { state, session_id, pane_id }) => {
                            self.apply_set_status(state, session_id, pane_id);
                        }
                        Some(SocketEvent::Flash { session_id, pane_id }) => {
                            self.apply_flash(session_id, pane_id);
                        }
                        Some(SocketEvent::NewSession { path }) => {
                            self.open_or_switch_session(path);
                        }
                        Some(SocketEvent::SetCwd { path }) => {
                            if let Some(session) = self.state.sessions.get_mut(self.state.active_session) {
                                if let Some(pane) = session.panes.get_mut(session.active_pane) {
                                    pane.cwd = path;
                                }
                            }
                        }
                        None => {
                            // Channel closed — socket server stopped, keep running
                        }
                    }
                    continue;
                }
                // PTY output
                maybe_pty = self.pty_rx.recv() => {
                    if let Some(out) = maybe_pty {
                        if out.exited {
                            self.close_pane_by_id(out.session_id, out.pane_id);
                            continue;
                        }
                        if let Some(pty) = self.ptys.get_mut(&(out.session_id, out.pane_id)) {
                            pty.process_output(&out.data);
                        }
                        // Auto-detect agent state from output (OSC takes priority)
                        if let Some(new_state) = crate::pty::parse_osc_state(&out.data)
                            .or_else(|| crate::pty::detect_agent_state(&out.data)) {
                            if let Some(session) = self.state.sessions.iter_mut()
                                .find(|s| s.id == out.session_id) {
                                if let Some(pane) = session.panes.iter_mut()
                                    .find(|p| p.id == out.pane_id) {
                                    pane.agent_state = new_state;
                                }
                            }
                        }
                    }
                    continue;
                }
            }

            // Check for terminal input after the sleep branch
            if event::poll(std::time::Duration::from_millis(0))? {
                let focus_before = self
                    .state
                    .sessions
                    .get(self.state.active_session)
                    .map(|s| (s.id, s.panes.get(s.active_pane).map(|p| p.id).unwrap_or(0)));

                match event::read()? {
                    Event::Resize(w, h) => {
                        self.layout_mode = layout::detect_layout(w);
                        let pty_rows = h.saturating_sub(2);
                        self.last_size = (pty_rows, w);
                        if let Some(client) = &self.daemon_client {
                            let writer = client.writer.clone();
                            tokio::spawn(async move {
                                let msg = serde_json::json!({
                                    "method": "resize",
                                    "params": { "rows": pty_rows, "cols": w }
                                })
                                .to_string()
                                    + "\n";
                                let mut wl = writer.lock().await;
                                let _ =
                                    tokio::io::AsyncWriteExt::write_all(&mut *wl, msg.as_bytes())
                                        .await;
                            });
                        } else {
                            for pty in self.ptys.values_mut() {
                                let _ = pty.resize(pty_rows, w);
                            }
                            for pty in self.client_ptys.values_mut() {
                                let _ = pty.resize(pty_rows, w);
                            }
                        }
                    }
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        // Rename prompt intercepts all keys when open
                        if let Some(ref mut input) = self.rename_input {
                            match key.code {
                                KeyCode::Esc => {
                                    self.rename_input = None;
                                }
                                KeyCode::Enter => {
                                    let new_name = input.trim().to_string();
                                    if !new_name.is_empty() {
                                        if let Some(session) =
                                            self.state.sessions.get_mut(self.state.active_session)
                                        {
                                            match self.rename_target {
                                                RenameTarget::Session => {
                                                    if let Some(ref mut project) = session.project {
                                                        project.name = new_name;
                                                    }
                                                }
                                                RenameTarget::Pane => {
                                                    if let Some(pane) =
                                                        session.panes.get_mut(session.active_pane)
                                                    {
                                                        pane.name = new_name;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    self.rename_input = None;
                                }
                                KeyCode::Backspace => {
                                    input.pop();
                                }
                                KeyCode::Char(c) => {
                                    input.push(c);
                                }
                                _ => {}
                            }
                            continue;
                        }

                        // Select popup intercepts all keys when open
                        if self.state.pending_select.is_some() {
                            if let Some(ref client) = self.daemon_client {
                                let writer = client.writer.clone();
                                let msg_opt: Option<String> = match key.code {
                                    KeyCode::Up | KeyCode::Char('k') => Some(
                                        serde_json::json!({
                                            "method": "select-nav",
                                            "params": {"delta": -1}
                                        })
                                        .to_string()
                                            + "\n",
                                    ),
                                    KeyCode::Down | KeyCode::Char('j') => Some(
                                        serde_json::json!({
                                            "method": "select-nav",
                                            "params": {"delta": 1}
                                        })
                                        .to_string()
                                            + "\n",
                                    ),
                                    KeyCode::Enter => Some(
                                        serde_json::json!({
                                            "method": "select-confirm",
                                            "params": {}
                                        })
                                        .to_string()
                                            + "\n",
                                    ),
                                    KeyCode::Esc | KeyCode::Char('q') => Some(
                                        serde_json::json!({
                                            "method": "select-cancel",
                                            "params": {}
                                        })
                                        .to_string()
                                            + "\n",
                                    ),
                                    _ => None,
                                };
                                if let Some(msg) = msg_opt {
                                    tokio::spawn(async move {
                                        let mut w = writer.lock().await;
                                        let _ = tokio::io::AsyncWriteExt::write_all(
                                            &mut *w,
                                            msg.as_bytes(),
                                        )
                                        .await;
                                    });
                                }
                            }
                            continue;
                        }

                        // Picker intercepts all keys when open
                        if self.show_picker {
                            match key.code {
                                KeyCode::Esc => {
                                    self.show_picker = false;
                                    self.picker = None;
                                }
                                KeyCode::Up => {
                                    if let Some(p) = &mut self.picker {
                                        p.move_up();
                                    }
                                }
                                KeyCode::Down => {
                                    if let Some(p) = &mut self.picker {
                                        p.move_down();
                                    }
                                }
                                KeyCode::Enter => {
                                    self.confirm_picker();
                                }
                                KeyCode::Backspace => {
                                    if let Some(p) = &mut self.picker {
                                        p.query.pop();
                                        let q = p.query.clone();
                                        p.update_query(q);
                                    }
                                }
                                KeyCode::Char(c) => {
                                    if let Some(p) = &mut self.picker {
                                        let mut q = p.query.clone();
                                        q.push(c);
                                        p.update_query(q);
                                    }
                                }
                                _ => {}
                            }
                            continue; // don't process as normal key
                        }

                        // Copy mode intercepts all keys when active
                        if self.copy_mode.is_some() {
                            let (sess_id, pane_id, scroll_offset) = {
                                let cm = self.copy_mode.as_ref().unwrap();
                                (cm.session_id, cm.pane_id, cm.scroll_offset)
                            };
                            let scrollback_len = self
                                .ptys
                                .get(&(sess_id, pane_id))
                                .map(|p| p.scrollback_lines().len())
                                .unwrap_or(0);
                            let visible_rows = self.last_size.0 as usize;
                            let max_offset = scrollback_len.saturating_sub(visible_rows);

                            match key.code {
                                KeyCode::Esc | KeyCode::Char('q') => {
                                    self.copy_mode = None;
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    self.copy_mode.as_mut().unwrap().scroll_offset =
                                        (scroll_offset + 1).min(max_offset);
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    self.copy_mode.as_mut().unwrap().scroll_offset =
                                        scroll_offset.saturating_sub(1);
                                }
                                KeyCode::PageUp => {
                                    self.copy_mode.as_mut().unwrap().scroll_offset =
                                        (scroll_offset + visible_rows / 2).min(max_offset);
                                }
                                KeyCode::PageDown => {
                                    self.copy_mode.as_mut().unwrap().scroll_offset =
                                        scroll_offset.saturating_sub(visible_rows / 2);
                                }
                                KeyCode::Char('g') => {
                                    self.copy_mode.as_mut().unwrap().scroll_offset = max_offset;
                                }
                                KeyCode::Char('G') => {
                                    self.copy_mode.as_mut().unwrap().scroll_offset = 0;
                                }
                                _ => {}
                            }
                            continue;
                        }

                        if key.code == KeyCode::Esc && self.show_dashboard {
                            self.show_dashboard = false;
                        } else if self.prefix_mode {
                            self.prefix_mode = false;
                            self.handle_prefix_key(key);
                        } else if is_ctrl_a(key) {
                            self.prefix_mode = true;
                        } else if let Some(bytes) = key_to_bytes(key) {
                            self.write_to_active_pty(&bytes);
                        }
                    }
                    Event::Mouse(mouse) => {
                        use crossterm::event::{MouseButton, MouseEventKind};
                        match mouse.kind {
                            MouseEventKind::Down(MouseButton::Left) => {
                                self.handle_mouse_click(mouse.column, mouse.row);
                            }
                            MouseEventKind::ScrollUp => {
                                self.handle_scroll(mouse.column, mouse.row, true);
                            }
                            MouseEventKind::ScrollDown => {
                                self.handle_scroll(mouse.column, mouse.row, false);
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }

                // If the focused pane changed, tell the daemon so it can clear Waiting state
                let focus_after = self
                    .state
                    .sessions
                    .get(self.state.active_session)
                    .map(|s| (s.id, s.panes.get(s.active_pane).map(|p| p.id).unwrap_or(0)));
                if focus_after != focus_before {
                    if let (Some((sess_id, pane_id)), Some(ref client)) =
                        (focus_after, &self.daemon_client)
                    {
                        let writer = client.writer.clone();
                        let msg = serde_json::json!({
                            "method": "focus-pane",
                            "params": { "session_id": sess_id, "pane_id": pane_id }
                        })
                        .to_string()
                            + "\n";
                        tokio::spawn(async move {
                            let mut w = writer.lock().await;
                            let _ =
                                tokio::io::AsyncWriteExt::write_all(&mut *w, msg.as_bytes()).await;
                        });
                    }
                }
            }
        }

        // Notify daemon of detach if applicable
        if self.detach_requested {
            if let Some(ref client) = self.daemon_client {
                let _ = client.detach().await;
            }
        }

        Ok(self.state)
    }

    fn handle_prefix_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            // ctrl-a after prefix: send literal ctrl-a to PTY
            KeyCode::Char('a') if ctrl => {
                self.write_to_active_pty(&[0x01]);
            }
            // next pane
            KeyCode::Char('n') => {
                if let Some(session) = self.state.sessions.get_mut(self.state.active_session) {
                    let len = session.panes.len();
                    if len > 0 {
                        session.active_pane = (session.active_pane + 1) % len;
                    }
                }
            }
            // prev pane
            KeyCode::Char('p') => {
                if let Some(session) = self.state.sessions.get_mut(self.state.active_session) {
                    let len = session.panes.len();
                    if len > 0 {
                        session.active_pane = (session.active_pane + len - 1) % len;
                    }
                }
            }
            // next session
            KeyCode::Char('N') => {
                let len = self.state.sessions.len();
                if len > 0 {
                    self.state.active_session = (self.state.active_session + 1) % len;
                }
            }
            // prev session
            KeyCode::Char('P') => {
                let len = self.state.sessions.len();
                if len > 0 {
                    self.state.active_session = (self.state.active_session + len - 1) % len;
                }
            }
            KeyCode::Char('d') => {
                self.detach_requested = true;
            }
            // Dashboard toggle
            KeyCode::Char('D') => {
                self.show_dashboard = !self.show_dashboard;
            }
            KeyCode::Char('z') => {
                self.zoomed = !self.zoomed;
            }
            // enter copy/scrollback mode
            KeyCode::Char('[') | KeyCode::PageUp => {
                self.enter_copy_mode();
            }
            // open project picker
            KeyCode::Char('f') => {
                self.open_picker();
            }
            // split pane (horizontal or vertical — both just add a pane for now)
            KeyCode::Char('"') | KeyCode::Char('%') => {
                self.add_pane();
            }
            // close active pane
            KeyCode::Char('x') => {
                self.close_active_pane();
            }
            // rename active pane
            KeyCode::Char(',') => {
                // seed with current pane name
                let current = self
                    .state
                    .sessions
                    .get(self.state.active_session)
                    .and_then(|s| s.panes.get(s.active_pane))
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                self.rename_target = RenameTarget::Pane;
                self.rename_input = Some(current);
            }
            // rename current session
            KeyCode::Char('S') => {
                let current = self
                    .state
                    .sessions
                    .get(self.state.active_session)
                    .and_then(|s| s.project.as_ref())
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                self.rename_target = RenameTarget::Session;
                self.rename_input = Some(current);
            }
            // jump to pane by number (1-9)
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let idx = (c as usize) - ('1' as usize); // '1' -> 0, '2' -> 1, etc.
                if let Some(session) = self.state.sessions.get_mut(self.state.active_session) {
                    if idx < session.panes.len() {
                        session.active_pane = idx;
                    }
                }
            }
            _ => {}
        }
    }

    fn enter_copy_mode(&mut self) {
        if let Some(session) = self.state.sessions.get(self.state.active_session) {
            if let Some(pane) = session.panes.get(session.active_pane) {
                self.copy_mode = Some(CopyMode {
                    session_id: session.id,
                    pane_id: pane.id,
                    scroll_offset: 0,
                });
            }
        }
    }

    fn open_picker(&mut self) {
        use crate::widgets::picker::{scan_projects, PickerState};
        let items = scan_projects(&self.state.sessions);
        self.picker = Some(PickerState::new(items));
        self.show_picker = true;
    }

    fn confirm_picker(&mut self) {
        let path = if let Some(picker) = &self.picker {
            picker.selected_item().map(|item| item.path.clone())
        } else {
            None
        };

        self.show_picker = false;
        self.picker = None;

        if let Some(path) = path {
            self.open_or_switch_session(path);
        }
    }

    fn open_or_switch_session(&mut self, path: PathBuf) {
        // If a session with this project root already exists, switch to it
        for (i, session) in self.state.sessions.iter().enumerate() {
            if session.project.as_ref().map(|p| &p.root) == Some(&path) {
                self.state.active_session = i;
                return;
            }
        }
        // Otherwise create a new session
        let project = jmux_core::detect_project(&path).or_else(|| {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("project")
                .to_string();
            Some(jmux_core::ProjectInfo {
                name,
                root: path.clone(),
                kind: jmux_core::ProjectKind::Plain,
            })
        });

        let new_session_id = self.state.sessions.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        let shell_name = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        let shell_bin = std::path::Path::new(&shell_name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("shell")
            .to_string();
        let pane = PaneState {
            id: 0,
            name: shell_bin,
            cwd: path.clone(),
            agent_state: jmux_core::AgentState::Idle,
            process_name: None,
            cmd: None,
        };
        let session = jmux_core::Session {
            id: new_session_id,
            project,
            panes: vec![pane],
            active_pane: 0,
        };
        self.state.sessions.push(session);
        self.state.active_session = self.state.sessions.len() - 1;

        // Spawn PTY for the new session's pane
        let (rows, cols) = self.last_size;
        let socket = self.socket_path.clone();
        let tx = self.pty_tx.clone();
        let new_sess = self.state.sessions.last().unwrap();
        let sess_id = new_sess.id;
        let pane_cwd = new_sess.panes[0].cwd.clone();
        if let Ok(pty) =
            crate::pty::PtyPane::spawn(sess_id, 0, &pane_cwd, rows, cols, &socket, tx, None)
        {
            self.ptys.insert((sess_id, 0), pty);
        }
    }

    fn add_pane(&mut self) {
        let session_idx = self.state.active_session;
        if let Some(session) = self.state.sessions.get(session_idx) {
            let session_id = session.id;
            let cwd = session
                .panes
                .get(session.active_pane)
                .map(|p| p.cwd.clone())
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

            if let Some(client) = &self.daemon_client {
                // Daemon mode — let the daemon spawn the PTY
                let writer = client.writer.clone();
                let cwd_str = cwd.to_string_lossy().to_string();
                tokio::spawn(async move {
                    let msg = serde_json::json!({
                        "method": "add-pane",
                        "params": { "session_id": session_id, "cwd": cwd_str }
                    })
                    .to_string()
                        + "\n";
                    let mut w = writer.lock().await;
                    let _ = tokio::io::AsyncWriteExt::write_all(&mut *w, msg.as_bytes()).await;
                });
                return; // daemon will broadcast updated state
            }

            // Standalone mode — spawn locally
            let new_id = session.panes.iter().map(|p| p.id).max().unwrap_or(0) + 1;
            let shell_name = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
            let shell_bin = std::path::Path::new(&shell_name)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("shell")
                .to_string();
            let new_pane = PaneState {
                id: new_id,
                name: shell_bin,
                cwd: cwd.clone(),
                agent_state: jmux_core::AgentState::Idle,
                process_name: None,
                cmd: None,
            };
            if let Some(session) = self.state.sessions.get_mut(session_idx) {
                session.panes.push(new_pane);
                session.active_pane = session.panes.len() - 1;
            }
            let (rows, cols) = self.last_size;
            if let Ok(pty) = crate::pty::PtyPane::spawn(
                session_id,
                new_id,
                &cwd,
                rows,
                cols,
                &self.socket_path,
                self.pty_tx.clone(),
                None,
            ) {
                self.ptys.insert((session_id, new_id), pty);
            }
        }
    }

    fn close_pane_by_id(&mut self, session_id: usize, pane_id: usize) {
        self.ptys.remove(&(session_id, pane_id));
        self.client_ptys.remove(&(session_id, pane_id));
        if let Some(client) = &self.daemon_client {
            let writer = client.writer.clone();
            tokio::spawn(async move {
                let msg = serde_json::json!({
                    "method": "close-pane",
                    "params": { "session_id": session_id, "pane_id": pane_id }
                })
                .to_string()
                    + "\n";
                let mut w = writer.lock().await;
                let _ = tokio::io::AsyncWriteExt::write_all(&mut *w, msg.as_bytes()).await;
            });
            return; // daemon will broadcast updated state
        }

        if let Some(sess_idx) = self.state.sessions.iter().position(|s| s.id == session_id) {
            let session = &mut self.state.sessions[sess_idx];
            if let Some(pane_idx) = session.panes.iter().position(|p| p.id == pane_id) {
                session.panes.remove(pane_idx);
                if session.active_pane >= session.panes.len() && !session.panes.is_empty() {
                    session.active_pane = session.panes.len() - 1;
                }
            }
            if self.state.sessions[sess_idx].panes.is_empty() {
                self.state.sessions.remove(sess_idx);
                if self.state.sessions.is_empty() {
                    self.quit_requested = true;
                } else if self.state.active_session >= self.state.sessions.len() {
                    self.state.active_session = self.state.sessions.len() - 1;
                }
            }
        }
    }

    fn close_active_pane(&mut self) {
        let session_idx = self.state.active_session;
        if let Some(session) = self.state.sessions.get_mut(session_idx) {
            let pane_id = session.panes[session.active_pane].id;
            let session_id = session.id;
            self.close_pane_by_id(session_id, pane_id);
        }
    }

    fn sync_pty_sizes(&mut self) {
        let rects = self.pane_rects.borrow().clone();
        for ((sess_id, pane_id), rect) in &rects {
            let cols = rect.width.saturating_sub(2).max(1);
            let rows = rect.height.saturating_sub(2).max(1);

            // Check BEFORE updating so we know whether to send to daemon
            let changed = self
                .ptys
                .get(&(*sess_id, *pane_id))
                .map(|p| p.cols != cols || p.rows != rows)
                .or_else(|| {
                    self.client_ptys
                        .get(&(*sess_id, *pane_id))
                        .map(|p| p.cols != cols || p.rows != rows)
                })
                .unwrap_or(true);

            if let Some(pty) = self.ptys.get_mut(&(*sess_id, *pane_id)) {
                if pty.cols != cols || pty.rows != rows {
                    let _ = pty.resize(rows, cols);
                }
            }
            if let Some(pty) = self.client_ptys.get_mut(&(*sess_id, *pane_id)) {
                if pty.cols != cols || pty.rows != rows {
                    let _ = pty.resize(rows, cols);
                }
            }

            if changed {
                if let Some(client) = &self.daemon_client {
                    let writer = client.writer.clone();
                    let sess_id = *sess_id;
                    let pane_id = *pane_id;
                    tokio::spawn(async move {
                        let msg = serde_json::json!({
                            "method": "resize-pane",
                            "params": { "session_id": sess_id, "pane_id": pane_id, "rows": rows, "cols": cols }
                        }).to_string() + "\n";
                        let mut w = writer.lock().await;
                        let _ = tokio::io::AsyncWriteExt::write_all(&mut *w, msg.as_bytes()).await;
                    });
                }
            }
        }
    }

    fn write_to_active_pty(&mut self, data: &[u8]) {
        let session_idx = self.state.active_session;
        if let Some(session) = self.state.sessions.get(session_idx) {
            let pane_idx = session.active_pane;
            if let Some(pane_state) = session.panes.get(pane_idx) {
                let key = (session.id, pane_state.id);
                if let Some(client) = &self.daemon_client {
                    let client = client.writer.clone();
                    let session_id = session.id;
                    let pane_id = pane_state.id;
                    let data = data.to_vec();
                    tokio::spawn(async move {
                        let msg = serde_json::json!({
                            "method": "input",
                            "params": {
                                "session_id": session_id,
                                "pane_id": pane_id,
                                "data": data,
                            }
                        })
                        .to_string()
                            + "\n";
                        let mut w = client.lock().await;
                        let _ = tokio::io::AsyncWriteExt::write_all(&mut *w, msg.as_bytes()).await;
                    });
                } else if let Some(pty) = self.ptys.get_mut(&key) {
                    let _ = pty.write_input(data);
                }
            }
        }
    }

    fn apply_set_status(
        &mut self,
        state: AgentState,
        session_id: Option<usize>,
        pane_id: Option<usize>,
    ) {
        let session = match session_id {
            Some(sid) => self.state.sessions.iter_mut().find(|s| s.id == sid),
            None => self.state.sessions.get_mut(self.state.active_session),
        };
        if let Some(session) = session {
            let pane = match pane_id {
                Some(pid) => session.panes.iter_mut().find(|p| p.id == pid),
                None => session.panes.get_mut(session.active_pane),
            };
            if let Some(pane) = pane {
                pane.agent_state = state;
            }
        }
    }

    fn apply_flash(&mut self, session_id: Option<usize>, pane_id: Option<usize>) {
        let session = match session_id {
            Some(sid) => self.state.sessions.iter_mut().find(|s| s.id == sid),
            None => self.state.sessions.get_mut(self.state.active_session),
        };
        if let Some(session) = session {
            let pane = match pane_id {
                Some(pid) => session.panes.iter_mut().find(|p| p.id == pid),
                None => session.panes.get_mut(session.active_pane),
            };
            if let Some(pane) = pane {
                pane.agent_state = AgentState::Waiting {
                    message: Some("Flash!".to_string()),
                };
            }
        }
    }

    fn handle_mouse_click(&mut self, col: u16, row: u16) {
        // Check sidebar pane row clicks first (more specific than session header)
        let sidebar_pane_rects = self.sidebar_pane_rects.borrow().clone();
        for ((sess_id, pane_id), rect) in &sidebar_pane_rects {
            if contains(rect, col, row) {
                if let Some((sess_idx, session)) = self
                    .state
                    .sessions
                    .iter_mut()
                    .enumerate()
                    .find(|(_, s)| s.id == *sess_id)
                {
                    if let Some(pane_idx) = session.panes.iter().position(|p| p.id == *pane_id) {
                        self.state.active_session = sess_idx;
                        session.active_pane = pane_idx;
                    }
                }
                return;
            }
        }

        // Check session header clicks
        let session_rects = self.session_rects.borrow().clone();
        for (i, rect) in session_rects.iter().enumerate() {
            if contains(rect, col, row) {
                self.state.active_session = i;
                return;
            }
        }

        // Check pane clicks
        let pane_rects = self.pane_rects.borrow().clone();
        for ((sess_id, pane_id), rect) in &pane_rects {
            if contains(rect, col, row) {
                if let Some((sess_idx, session)) = self
                    .state
                    .sessions
                    .iter_mut()
                    .enumerate()
                    .find(|(_, s)| s.id == *sess_id)
                {
                    if let Some(pane_idx) = session.panes.iter().position(|p| p.id == *pane_id) {
                        self.state.active_session = sess_idx;
                        session.active_pane = pane_idx;
                    }
                }
                return;
            }
        }
    }

    fn handle_scroll(&mut self, col: u16, row: u16, up: bool) {
        let pane_rects = self.pane_rects.borrow().clone();
        for ((sess_id, pane_id), rect) in &pane_rects {
            if contains(rect, col, row) {
                let scroll_bytes: &[u8] = if up { b"\x1b[5~" } else { b"\x1b[6~" };
                if let Some(pty) = self.ptys.get_mut(&(*sess_id, *pane_id)) {
                    let _ = pty.write_input(scroll_bytes);
                }
                return;
            }
        }
    }

    fn render(&self, f: &mut ratatui::Frame) {
        match self.layout_mode {
            LayoutMode::Wide => layout::render_wide(f, self),
            LayoutMode::Narrow => layout::render_narrow(f, self),
        }
        if let Some(ref sel) = self.state.pending_select {
            crate::widgets::select_popup::render_select_popup(f, sel);
        }
    }

    #[cfg(test)]
    pub fn apply_set_status_pub(&mut self, state: AgentState) {
        self.apply_set_status(state, None, None);
    }

    #[cfg(test)]
    pub fn apply_flash_pub(&mut self) {
        self.apply_flash(None, None);
    }

    #[cfg(test)]
    pub fn close_active_pane_pub(&mut self) {
        self.close_active_pane();
    }

    #[cfg(test)]
    pub fn enter_copy_mode_pub(&mut self) {
        self.enter_copy_mode();
    }

    #[cfg(test)]
    pub fn open_or_switch_session_pub(&mut self, path: std::path::PathBuf) {
        self.open_or_switch_session(path);
    }
}

/// Get the process name for a PID using the platform's ps command.
fn process_name(pid: u32) -> Option<String> {
    let out = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if name.is_empty() {
        None
    } else {
        // Strip path prefix — just the binary name
        std::path::Path::new(&name)
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
    }
}

fn is_ctrl_a(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('a')
}

fn contains(rect: &Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
}
