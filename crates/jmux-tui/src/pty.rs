use anyhow::Result;
use jmux_core::AgentState;
use portable_pty::{native_pty_system, unix::RawFd, CommandBuilder, MasterPty, PtySize};
use std::collections::VecDeque;
use std::path::PathBuf;
use tokio::sync::mpsc;

const SCROLLBACK_MAX: usize = 5000;

/// Null PTY master — used for client-mode panes fed from daemon output.
struct NullMaster;
impl MasterPty for NullMaster {
    fn resize(&self, _: PtySize) -> anyhow::Result<()> {
        Ok(())
    }
    fn get_size(&self) -> anyhow::Result<PtySize> {
        Ok(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
    }
    fn try_clone_reader(&self) -> anyhow::Result<Box<dyn std::io::Read + Send>> {
        Ok(Box::new(std::io::empty()))
    }
    fn take_writer(&self) -> anyhow::Result<Box<dyn std::io::Write + Send>> {
        Ok(Box::new(std::io::sink()))
    }
    fn process_group_leader(&self) -> Option<libc::pid_t> {
        None
    }
    fn as_raw_fd(&self) -> Option<RawFd> {
        None
    }
}

pub struct PtyOutput {
    pub session_id: usize,
    pub pane_id: usize,
    pub data: Vec<u8>,
    pub exited: bool,
}

pub struct PtyPane {
    pub master: Box<dyn MasterPty + Send>,
    pub writer: Box<dyn std::io::Write + Send>,
    pub parser: vt100::Parser,
    pub rows: u16,
    pub cols: u16,
    pub scrollback: VecDeque<String>,
    /// PID of the shell process we spawned (None for client panes).
    pub shell_pid: Option<u32>,
    /// Last observed foreground PID — used to detect process transitions.
    last_fg_pid: Option<u32>,
}

impl PtyPane {
    // Spawn a shell (or custom command) in a new PTY. Starts a background thread to read output
    // and send it as PtyOutput on `tx`.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        session_id: usize,
        pane_id: usize,
        cwd: &PathBuf,
        rows: u16,
        cols: u16,
        socket_path: &str,
        tx: mpsc::Sender<PtyOutput>,
        cmd: Option<&str>,
    ) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let (bin, arg) = if let Some(cmd_str) = cmd {
            let parts: Vec<&str> = cmd_str.splitn(2, ' ').collect();
            (
                parts[0].to_string(),
                parts.get(1).copied().map(|s| s.to_string()),
            )
        } else {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
            (shell, None)
        };

        let mut cmd = CommandBuilder::new(&bin);
        if let Some(a) = arg {
            cmd.arg(a);
        }
        cmd.cwd(cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("JMUX_SOCKET", socket_path);
        cmd.env("JMUX_SESSION_ID", session_id.to_string());
        cmd.env("JMUX_PANE_ID", pane_id.to_string());

        let child = pair.slave.spawn_command(cmd)?;
        let shell_pid = child.process_id();

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        // Reader runs in a blocking thread (PTY I/O is blocking)
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match std::io::Read::read(&mut reader, &mut buf) {
                    Ok(0) | Err(_) => {
                        let _ = tx.blocking_send(PtyOutput {
                            session_id,
                            pane_id,
                            data: vec![],
                            exited: true,
                        });
                        break;
                    }
                    Ok(n) => {
                        let data = buf[..n].to_vec();
                        if tx
                            .blocking_send(PtyOutput {
                                session_id,
                                pane_id,
                                data,
                                exited: false,
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        });

        Ok(Self {
            master: pair.master,
            writer,
            parser: vt100::Parser::new(rows, cols, 0),
            rows,
            cols,
            scrollback: VecDeque::new(),
            shell_pid,
            last_fg_pid: None,
        })
    }

    /// Create a parser-only pane with no real PTY — used in daemon client mode.
    /// The master and writer are no-ops; bytes are fed via `process_output`.
    pub fn new_client(rows: u16, cols: u16) -> Self {
        Self {
            master: Box::new(NullMaster),
            writer: Box::new(std::io::sink()),
            parser: vt100::Parser::new(rows, cols, 0),
            rows,
            cols,
            scrollback: VecDeque::new(),
            shell_pid: None,
            last_fg_pid: None,
        }
    }

    /// Check if the foreground process changed since last call.
    /// Returns `(is_running, Option<new_process_name>)`.
    /// `new_process_name` is `Some` only when the foreground process changed.
    pub fn poll_fg_process(&mut self) -> (bool, Option<String>) {
        let fg_pid = self.master.process_group_leader().map(|p| p as u32);
        let running = match (fg_pid, self.shell_pid) {
            (Some(fg), Some(shell)) => fg != shell,
            (Some(_), None) => true,
            _ => false,
        };

        if fg_pid == self.last_fg_pid {
            return (running, None);
        }
        self.last_fg_pid = fg_pid;

        let name = if running {
            fg_pid.and_then(fg_process_name)
        } else {
            None // back to shell — caller decides the label
        };
        (running, Some(name.unwrap_or_default()))
    }

    /// Returns true if a process other than the shell is in the PTY foreground.
    pub fn is_process_running(&self) -> bool {
        match (
            self.master.process_group_leader().map(|p| p as u32),
            self.shell_pid,
        ) {
            (Some(fg), Some(shell)) => fg != shell,
            (Some(_), None) => true,
            _ => false,
        }
    }

    pub fn process_output(&mut self, data: &[u8]) {
        self.parser.process(data);
        self.append_scrollback(data);
    }

    fn append_scrollback(&mut self, data: &[u8]) {
        let text = strip_ansi(data);
        for c in text.chars() {
            if self.scrollback.is_empty() {
                self.scrollback.push_back(String::new());
            }
            if c == '\n' || c == '\r' {
                if c == '\n' {
                    self.scrollback.push_back(String::new());
                }
            } else {
                self.scrollback.back_mut().unwrap().push(c);
            }
        }
        while self.scrollback.len() > SCROLLBACK_MAX {
            self.scrollback.pop_front();
        }
    }

    pub fn scrollback_lines(&self) -> &VecDeque<String> {
        &self.scrollback
    }

    pub fn write_input(&mut self, data: &[u8]) -> Result<()> {
        use std::io::Write;
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        self.rows = rows;
        self.cols = cols;
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        self.parser.set_size(rows, cols);
        Ok(())
    }
}

fn fg_process_name(pid: u32) -> Option<String> {
    let out = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn strip_ansi(data: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < data.len() {
        if data[i] == 0x1b {
            i += 1;
            if i < data.len() && data[i] == b'[' {
                // CSI sequence: skip until letter
                i += 1;
                while i < data.len() && !data[i].is_ascii_alphabetic() {
                    i += 1;
                }
                i += 1; // skip the final byte
            } else if i < data.len() && data[i] == b']' {
                // OSC sequence: skip until ST (0x07 or ESC \)
                i += 1;
                while i < data.len() && data[i] != 0x07 && data[i] != 0x1b {
                    i += 1;
                }
                if i < data.len() && data[i] == 0x1b {
                    i += 2;
                } else {
                    i += 1;
                }
            } else {
                i += 1; // skip single ESC sequence char
            }
        } else if data[i] >= 0x20 || data[i] == b'\n' || data[i] == b'\r' || data[i] == b'\t' {
            out.push(data[i] as char);
            i += 1;
        } else {
            i += 1; // skip other control chars
        }
    }
    out
}

/// Parse jmux OSC state sequence: ESC ] 9999 ; state=<state> [;message=<msg>] BEL/ST
/// Tools emit this to set agent state without needing JMUX_SOCKET.
/// Example: printf '\e]9999;state=working\a'
///          printf '\e]9999;state=waiting;message=needs input\a'
pub fn parse_osc_state(data: &[u8]) -> Option<AgentState> {
    let text = String::from_utf8_lossy(data);
    // Find ESC ] 9999 ;
    let marker = "\x1b]9999;";
    let start = text.find(marker)?;
    let after = &text[start + marker.len()..];
    // Find terminator: BEL (0x07) or ESC \ (ST)
    let end = after.find('\x07').or_else(|| after.find("\x1b\\"))?;
    let payload = &after[..end];

    let mut state_str = None;
    let mut message = None;
    for part in payload.split(';') {
        if let Some(v) = part.strip_prefix("state=") {
            state_str = Some(v);
        } else if let Some(v) = part.strip_prefix("message=") {
            message = Some(v.to_string());
        }
    }

    match state_str? {
        "working" => Some(AgentState::Working { message }),
        "waiting" => Some(AgentState::Waiting { message }),
        "idle" => Some(AgentState::Idle),
        "error" => Some(AgentState::Error { message }),
        _ => None,
    }
}

pub fn detect_agent_state(data: &[u8]) -> Option<AgentState> {
    let text = String::from_utf8_lossy(data);

    // Claude Code is handled via hooks (jmux set-status) — no pattern matching needed.

    // ── OpenAI Codex CLI ─────────────────────────────────────────────────────
    if text.contains("Running...") || text.contains("Applying") {
        return Some(AgentState::Working { message: None });
    }
    if text.contains("Approve?") || text.contains("Allow this action") {
        return Some(AgentState::Waiting {
            message: Some("approve action".to_string()),
        });
    }

    // ── Aider ────────────────────────────────────────────────────────────────
    if text.contains("Add these files to the chat?")
        || text.contains("Run shell commands?")
        || text.contains("Create new file?")
    {
        return Some(AgentState::Waiting {
            message: Some("aider needs input".to_string()),
        });
    }
    if text.contains("tokens sent") || (text.contains("cost") && text.contains("session")) {
        return Some(AgentState::Idle);
    }

    // ── Gemini CLI ───────────────────────────────────────────────────────────
    if text.contains("Gemini is thinking") {
        return Some(AgentState::Working { message: None });
    }

    // ── GitHub Copilot CLI ───────────────────────────────────────────────────
    if text.contains("Copilot is thinking") {
        return Some(AgentState::Working { message: None });
    }
    if text.contains("? Allow") && text.contains("command") {
        return Some(AgentState::Waiting {
            message: Some("approve command".to_string()),
        });
    }

    // ── Generic tool-use / MCP patterns ──────────────────────────────────────
    if text.contains("Calling tool")
        || text.contains("Tool call:")
        || text.contains("Function call:")
        || text.contains("Executing tool")
    {
        return Some(AgentState::Working {
            message: Some("tool call".to_string()),
        });
    }
    if text.contains("Tool requires approval") {
        return Some(AgentState::Waiting {
            message: Some("approve tool".to_string()),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- detect_agent_state ---

    #[test]
    fn detects_claude_working() {
        let data = b"esc to interrupt";
        assert!(matches!(
            detect_agent_state(data),
            Some(AgentState::Working { .. })
        ));
    }

    #[test]
    fn detects_claude_working_capitalized() {
        assert!(matches!(
            detect_agent_state(b"Esc to interrupt"),
            Some(AgentState::Working { .. })
        ));
    }

    #[test]
    fn detects_waiting_yn_prompt() {
        assert!(matches!(
            detect_agent_state(b"Do you want to proceed (y/n)?"),
            Some(AgentState::Waiting { .. })
        ));
    }

    #[test]
    fn detects_codex_working() {
        assert!(matches!(
            detect_agent_state(b"Running..."),
            Some(AgentState::Working { .. })
        ));
    }

    #[test]
    fn detects_codex_approve() {
        assert!(matches!(
            detect_agent_state(b"Approve? (yes/no)"),
            Some(AgentState::Waiting { .. })
        ));
    }

    #[test]
    fn detects_done() {
        assert!(matches!(
            detect_agent_state(b"\xe2\x9c\x93 Done"),
            Some(AgentState::Idle)
        ));
    }

    #[test]
    fn returns_none_for_plain_output() {
        assert!(detect_agent_state(b"hello world\n$ ").is_none());
    }

    // --- strip_ansi ---

    #[test]
    fn strips_csi_color_codes() {
        let input = b"\x1b[32mgreen text\x1b[0m";
        let result = strip_ansi(input);
        assert_eq!(result, "green text");
    }

    #[test]
    fn strips_osc_sequences() {
        // OSC title sequence
        let input = b"\x1b]0;my title\x07plain";
        let result = strip_ansi(input);
        assert_eq!(result, "plain");
    }

    #[test]
    fn preserves_newlines() {
        let input = b"line1\nline2\n";
        let result = strip_ansi(input);
        assert_eq!(result, "line1\nline2\n");
    }

    #[test]
    fn handles_empty_input() {
        assert_eq!(strip_ansi(b""), "");
    }

    #[test]
    fn strip_ansi_mixed() {
        let input = b"\x1b[1;32mBold green\x1b[0m normal \x1b[31mred\x1b[0m";
        let result = strip_ansi(input);
        assert_eq!(result, "Bold green normal red");
    }
}
