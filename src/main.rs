use std::io;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use jmux_core::persistence::{SavedSession, SavedState};
use jmux_core::{AgentState, AppState, PaneState, Session};
use jmux_tui::{run_daemon, run_socket_server, App, DaemonClient};
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::sync::mpsc;

#[derive(Parser)]
#[command(
    name = "jmux",
    about = "Terminal multiplexer with project and agent awareness"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Attach,
    Daemon,
    SetStatus {
        state: String,
        message: Option<String>,
    },
    Flash,
    Ls,
    Setup {
        agent: String,
    },
    New {
        path: Option<String>,
    },
    Init {
        shell: Option<String>,
    },
    SetCwd {
        path: String,
    },
    SetName {
        name: String,
    },
    Kill {
        name: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Command::Attach) {
        Command::Attach => run_tui().await?,
        Command::Daemon => cmd_daemon().await?,
        Command::SetStatus { state, message } => {
            let Ok(socket_path) = get_socket_path() else {
                return Ok(()); // not in a jmux session — silently succeed
            };
            let session_id = std::env::var("JMUX_SESSION_ID")
                .ok()
                .and_then(|s| s.parse::<u64>().ok());
            let pane_id = std::env::var("JMUX_PANE_ID")
                .ok()
                .and_then(|s| s.parse::<u64>().ok());
            let req = serde_json::json!({
                "method": "set-status",
                "params": {
                    "state": state,
                    "message": message,
                    "session_id": session_id,
                    "pane_id": pane_id,
                },
            });
            let _ = send_socket_message(&socket_path, &req.to_string()).await;
        }
        Command::Flash => {
            let Ok(socket_path) = get_socket_path() else {
                return Ok(()); // not in a jmux session — silently succeed
            };
            let session_id = std::env::var("JMUX_SESSION_ID")
                .ok()
                .and_then(|s| s.parse::<u64>().ok());
            let pane_id = std::env::var("JMUX_PANE_ID")
                .ok()
                .and_then(|s| s.parse::<u64>().ok());
            let req = serde_json::json!({
                "method": "flash",
                "params": {
                    "session_id": session_id,
                    "pane_id": pane_id,
                },
            });
            let _ = send_socket_message(&socket_path, &req.to_string()).await;
        }
        Command::Ls => {
            cmd_ls()?;
        }
        Command::Setup { agent } => match agent.to_lowercase().as_str() {
            "claude" => setup_claude()?,
            "codex" => setup_codex()?,
            "aider" => setup_agent_wrapper("aider", &["aider"])?,
            "gemini" => setup_agent_wrapper("gemini", &["gemini"])?,
            "all" => {
                setup_claude()?;
                for name in &["codex", "aider", "gemini"] {
                    if which_bin(name).is_some() {
                        setup_agent_wrapper(name, &[name])?;
                    }
                }
            }
            other => anyhow::bail!(
                "unknown agent: {}. Supported: claude, codex, aider, gemini, all",
                other
            ),
        },
        Command::New { path } => {
            let target = path
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            // If inside a running jmux session, send a socket command to open a new session there
            // Otherwise just exec jmux attach in that directory
            if let Ok(socket_path) = get_socket_path() {
                let req = serde_json::json!({
                    "method": "new-session",
                    "params": { "path": target.to_string_lossy() }
                });
                send_socket_message(&socket_path, &req.to_string()).await?;
                println!("opened new session for {}", target.display());
            } else {
                // Not inside jmux — cd to path and run attach
                std::env::set_current_dir(&target)?;
                run_tui().await?;
            }
        }
        Command::Init { shell } => {
            let shell = shell.as_deref().unwrap_or("zsh");
            match shell {
                "zsh" | "bash" => print!("{}", init_script(shell)),
                other => anyhow::bail!("unsupported shell: {}. Use: zsh, bash", other),
            }
        }
        Command::Kill { name } => {
            // Tell running daemon to close the session, then clean up saved state
            let state = build_state();
            let daemon_path = daemon_socket_path(&state);
            if daemon_path.exists() && tokio::net::UnixStream::connect(&daemon_path).await.is_ok() {
                let req = serde_json::json!({
                    "method": "kill-session",
                    "params": { "name": &name }
                });
                let _ = send_socket_message(&daemon_path, &req.to_string()).await;
            }
            jmux_core::persistence::kill_session(&name)?;
        }
        Command::SetCwd { path } => {
            if let Ok(socket_path) = get_socket_path() {
                let req = serde_json::json!({
                    "method": "set-cwd",
                    "params": { "path": path }
                });
                let _ = send_socket_message(&socket_path, &req.to_string()).await;
            }
        }
        Command::SetName { name } => {
            let Ok(socket_path) = get_socket_path() else {
                return Ok(());
            };
            let session_id = std::env::var("JMUX_SESSION_ID")
                .ok()
                .and_then(|s| s.parse::<u64>().ok());
            let pane_id = std::env::var("JMUX_PANE_ID")
                .ok()
                .and_then(|s| s.parse::<u64>().ok());
            let req = serde_json::json!({
                "method": "set-name",
                "params": { "name": name, "session_id": session_id, "pane_id": pane_id }
            });
            let _ = send_socket_message(&socket_path, &req.to_string()).await;
        }
    }

    Ok(())
}

fn get_socket_path() -> Result<PathBuf> {
    std::env::var("JMUX_SOCKET")
        .map(PathBuf::from)
        .map_err(|_| anyhow!("not inside a jmux session (JMUX_SOCKET not set)"))
}

fn daemon_socket_path(state: &AppState) -> PathBuf {
    let key = state
        .sessions
        .first()
        .and_then(|s| s.project.as_ref())
        .map(|p| p.root.to_string_lossy().to_string())
        .unwrap_or_else(|| "default".to_string());
    // Simple hash to get a stable, short filename
    let hash = key
        .bytes()
        .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
    PathBuf::from(format!("/tmp/jmux-daemon-{:016x}.sock", hash))
}

async fn cmd_daemon() -> Result<()> {
    let state = build_state();
    let socket_path = daemon_socket_path(&state);

    if socket_path.exists() {
        // Check if daemon is already alive
        if tokio::net::UnixStream::connect(&socket_path).await.is_ok() {
            println!("jmux daemon already running");
            println!("socket: {}", socket_path.display());
            println!("run 'jmux attach' to connect");
            return Ok(());
        }
        // Stale socket — remove it
        let _ = std::fs::remove_file(&socket_path);
    }

    // Set JMUX_SOCKET so pane child processes report to this daemon
    std::env::set_var("JMUX_SOCKET", &socket_path);

    let size = crossterm::terminal::size().unwrap_or((200, 50));
    let cols = std::env::var("JMUX_INITIAL_COLS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(size.0);
    let rows = std::env::var("JMUX_INITIAL_ROWS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(size.1.saturating_sub(2));

    println!("jmux daemon starting…");
    println!("socket: {}", socket_path.display());
    println!("press ctrl-c to stop\n");

    run_daemon(state, &socket_path, rows, cols).await?;

    let _ = std::fs::remove_file(&socket_path);
    Ok(())
}

async fn send_socket_message(path: &PathBuf, message: &str) -> Result<()> {
    let mut stream = UnixStream::connect(path).await?;
    stream.write_all(message.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await?;
    Ok(())
}

fn init_script(shell: &str) -> &'static str {
    match shell {
        "zsh" => include_str!("../jmux-init.zsh"),
        _ => include_str!("../jmux-init.bash"),
    }
}

async fn run_tui() -> Result<()> {
    let state = build_state();
    let daemon_path = daemon_socket_path(&state);

    // Connect to existing daemon if already running and not outdated
    if daemon_path.exists() {
        let binary_newer = std::env::current_exe().ok().and_then(|exe| {
            std::fs::metadata(&exe)
                .ok()?
                .modified()
                .ok()
                .zip(std::fs::metadata(&daemon_path).ok()?.modified().ok())
                .map(|(exe_t, sock_t)| exe_t > sock_t)
        });

        if binary_newer == Some(true) {
            // Rebuilt binary — kill old daemon so we start fresh
            eprintln!("jmux: new binary detected, restarting daemon...");
            let _ = std::fs::remove_file(&daemon_path);
        } else if let Ok(client) = DaemonClient::connect(&daemon_path).await {
            // Subscribe first so the background reader receives the daemon's initial state
            let _ = client.subscribe().await;
            // Give the async reader a moment to parse the state response
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;

            let needs_new_session = match client.current_state() {
                Some(s) => s.sessions.is_empty(),
                None => true,
            };
            if needs_new_session {
                let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
                let req = serde_json::json!({
                    "method": "new-session",
                    "params": { "path": cwd.to_string_lossy() }
                });
                let _ = send_socket_message(&daemon_path, &req.to_string()).await;
                // Brief pause so daemon processes the new-session before TUI subscribes
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            return run_tui_client(state, client, &daemon_path).await;
        } else {
            // Stale socket
            let _ = std::fs::remove_file(&daemon_path);
        }
    }

    // No daemon running — spawn one as a background process then attach
    let exe = std::env::current_exe()?;
    let term_size = crossterm::terminal::size().unwrap_or((200, 50));
    std::process::Command::new(&exe)
        .arg("daemon")
        .env("JMUX_INITIAL_COLS", term_size.0.to_string())
        .env(
            "JMUX_INITIAL_ROWS",
            term_size.1.saturating_sub(2).to_string(),
        )
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    // Wait up to 2s for the daemon socket to appear
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if daemon_path.exists() {
            if let Ok(client) = DaemonClient::connect(&daemon_path).await {
                return run_tui_client(state, client, &daemon_path).await;
            }
        }
    }

    anyhow::bail!("daemon did not start in time — try running `jmux daemon` manually")
}

async fn run_tui_client(
    state: AppState,
    client: DaemonClient,
    daemon_path: &std::path::Path,
) -> Result<()> {
    // Use daemon socket path as the effective JMUX_SOCKET
    let socket_path_str = daemon_path.to_string_lossy().to_string();

    // Keep sender alive so the channel stays open (dropping it would close rx and spam select!)
    let (_tx, rx) = mpsc::channel::<jmux_tui::SocketEvent>(1);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let final_state = App::new_daemon_client(state, client, rx, socket_path_str)
        .run(&mut terminal)
        .await?;

    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    disable_raw_mode()?;
    terminal.show_cursor()?;

    // Don't save state — daemon owns it
    let _ = final_state;
    Ok(())
}

async fn run_tui_standalone(state: AppState) -> Result<()> {
    let pid = std::process::id();
    let socket_path = PathBuf::from(format!("/tmp/jmux-{}.sock", pid));

    // Set JMUX_SOCKET so child processes can find the socket
    std::env::set_var("JMUX_SOCKET", &socket_path);

    let (tx, rx) = mpsc::channel::<jmux_tui::SocketEvent>(64);

    let socket_path_clone = socket_path.clone();
    tokio::spawn(async move {
        if let Err(e) = run_socket_server(socket_path_clone, tx).await {
            eprintln!("jmux socket server error: {}", e);
        }
    });

    let socket_path_str = socket_path.to_string_lossy().to_string();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let final_state = App::new(state, rx, socket_path_str)
        .run(&mut terminal)
        .await?;

    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    disable_raw_mode()?;
    terminal.show_cursor()?;

    let saved = to_saved_state(&final_state);
    let _ = jmux_core::persistence::save(&saved);

    let _ = std::fs::remove_file(&socket_path);
    Ok(())
}

fn build_state() -> AppState {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let project = jmux_core::detect_project(&cwd);
    let project_root = project
        .as_ref()
        .map(|p| p.root.clone())
        .unwrap_or(cwd.clone());

    // Load .jmux.toml if present
    let config = jmux_core::load_config(&project_root);

    let saved = jmux_core::persistence::load();

    let panes = if let Some(config) = &config {
        if !config.panes.is_empty() {
            // Build panes from config
            config
                .panes
                .iter()
                .enumerate()
                .map(|(i, pane_cfg)| PaneState {
                    id: i,
                    name: pane_cfg.name.clone(),
                    cwd: project_root.clone(),
                    agent_state: AgentState::Idle,
                    process_name: None,
                    cmd: pane_cfg.cmd.clone(),
                })
                .collect()
        } else {
            default_panes(&project_root, &saved, project.as_ref())
        }
    } else {
        default_panes(&project_root, &saved, project.as_ref())
    };

    // Use config project name override if provided
    let final_project = if let Some(name) = config
        .as_ref()
        .and_then(|c| c.project.as_ref())
        .and_then(|p| p.name.as_ref())
    {
        project
            .map(|mut p| {
                p.name = name.clone();
                p
            })
            .or_else(|| {
                Some(jmux_core::ProjectInfo {
                    name: name.clone(),
                    root: project_root.clone(),
                    kind: jmux_core::ProjectKind::Plain,
                })
            })
    } else {
        project
    };

    let session = Session {
        id: 0,
        project: final_project,
        active_pane: 0,
        panes,
    };

    AppState {
        sessions: vec![session],
        active_session: 0,
    }
}

fn default_panes(
    pane_cwd: &std::path::Path,
    saved: &jmux_core::persistence::SavedState,
    project: Option<&jmux_core::ProjectInfo>,
) -> Vec<PaneState> {
    let project_root: Option<PathBuf> = project.map(|p| p.root.clone());
    if let Some(saved_session) = jmux_core::persistence::find_session(saved, &project_root) {
        if !saved_session.pane_cwds.is_empty() {
            return saved_session
                .pane_cwds
                .iter()
                .enumerate()
                .map(|(i, saved_cwd)| PaneState {
                    id: i,
                    name: "shell".to_string(),
                    cwd: saved_cwd.clone(),
                    agent_state: AgentState::Idle,
                    process_name: None,
                    cmd: None,
                })
                .collect();
        }
    }
    vec![PaneState {
        id: 0,
        name: "shell".to_string(),
        cwd: pane_cwd.to_path_buf(),
        agent_state: AgentState::Idle,
        process_name: None,
        cmd: None,
    }]
}

fn to_saved_state(state: &AppState) -> SavedState {
    let sessions = state
        .sessions
        .iter()
        .map(|s| {
            let name = s
                .project
                .as_ref()
                .map(|p| p.name.clone())
                .unwrap_or_else(|| format!("session-{}", s.id));
            let project_root = s.project.as_ref().map(|p| p.root.clone());
            let pane_cwds = s.panes.iter().map(|p| p.cwd.clone()).collect();
            SavedSession {
                name,
                project_root,
                pane_cwds,
            }
        })
        .collect();
    SavedState { sessions }
}

const CYAN: &str = "\x1b[36m";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";
const GREEN: &str = "\x1b[32m";

fn cmd_ls() -> Result<()> {
    let saved = jmux_core::persistence::load();

    if saved.sessions.is_empty() {
        println!("no sessions saved");
        return Ok(());
    }

    // Compute column widths
    let header_session = "SESSION";
    let header_project = "PROJECT";
    let header_panes = "PANES";
    let header_status = "STATUS";

    let max_session = saved
        .sessions
        .iter()
        .map(|s| s.name.len())
        .max()
        .unwrap_or(0)
        .max(header_session.len());

    let max_project = saved
        .sessions
        .iter()
        .map(|s| {
            s.project_root
                .as_ref()
                .and_then(|r| r.file_name())
                .and_then(|n| n.to_str())
                .map(|n| n.len())
                .unwrap_or(1)
        }) // "—" is 1 char (though 3 bytes)
        .max()
        .unwrap_or(0)
        .max(header_project.len());

    let max_panes = saved
        .sessions
        .iter()
        .map(|s| s.pane_cwds.len().to_string().len())
        .max()
        .unwrap_or(0)
        .max(header_panes.len());

    let status_text = "idle (last session)";
    let max_status = status_text.len().max(header_status.len());

    let sep_width = 2 + max_session + 2 + max_project + 2 + max_panes + 2 + max_status;

    // Header
    println!(
        "  {BOLD}{:<sw$}  {:<pw$}  {:<nw$}  {:<stw$}{RESET}",
        header_session,
        header_project,
        header_panes,
        header_status,
        sw = max_session,
        pw = max_project,
        nw = max_panes,
        stw = max_status,
    );
    println!("  {DIM}{}{RESET}", "─".repeat(sep_width));

    for s in &saved.sessions {
        let project = s
            .project_root
            .as_ref()
            .and_then(|r| r.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("—");
        let pane_count = s.pane_cwds.len();

        println!(
            "  {CYAN}{:<sw$}{RESET}  {:<pw$}  {:<nw$}  {GREEN}{}{RESET}",
            s.name,
            project,
            pane_count,
            status_text,
            sw = max_session,
            pw = max_project,
            nw = max_panes,
        );
    }

    let n = saved.sessions.len();
    println!(
        "\n  {DIM}{} session(s) — run 'jmux attach' to open{RESET}",
        n
    );

    Ok(())
}

fn setup_codex() -> Result<()> {
    let home = dirs_next::home_dir().ok_or_else(|| anyhow::anyhow!("cannot find home dir"))?;

    // Find the real codex binary (avoid wrapping ourselves)
    let real_codex = find_real_codex(&home)?;

    let bin_dir = home.join(".local").join("bin");
    std::fs::create_dir_all(&bin_dir)?;

    let wrapper_path = bin_dir.join("codex");

    let wrapper = format!(
        r#"#!/usr/bin/env bash
# jmux codex wrapper — auto-generated by jmux setup codex
# Real codex binary: {real_codex}

if [ -n "$JMUX_SOCKET" ]; then
    jmux set-status working "Codex running" 2>/dev/null
fi

"{real_codex}" "$@"
EXIT_CODE=$?

if [ -n "$JMUX_SOCKET" ]; then
    if [ $EXIT_CODE -eq 0 ]; then
        jmux set-status idle 2>/dev/null
    else
        jmux set-status error "Codex exited ($EXIT_CODE)" 2>/dev/null
    fi
fi

exit $EXIT_CODE
"#,
        real_codex = real_codex
    );

    std::fs::write(&wrapper_path, &wrapper)?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper_path, std::fs::Permissions::from_mode(0o755))?;
    }

    println!("Created codex wrapper at {}", wrapper_path.display());
    println!();

    // Check if ~/.local/bin is in PATH
    let in_path = std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .any(|p| p == bin_dir.to_string_lossy().as_ref());

    if !in_path {
        println!("⚠️  Add ~/.local/bin to your PATH:");
        println!("   echo 'export PATH=\"$HOME/.local/bin:$PATH\"' >> ~/.zshrc");
        println!("   source ~/.zshrc");
        println!();
    }

    println!("✓ Codex will now update jmux pane borders when running inside a jmux session.");
    println!("  Start codex normally — the wrapper is transparent when outside jmux.");

    Ok(())
}

fn which_bin(name: &str) -> Option<String> {
    let path_var = std::env::var("PATH").unwrap_or_default();
    for dir in path_var.split(':') {
        let candidate = std::path::Path::new(dir).join(name);
        if candidate.exists() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    None
}

/// Generic agent wrapper: creates ~/.local/bin/{name} that reports state to jmux.
fn setup_agent_wrapper(name: &str, cmd_candidates: &[&str]) -> Result<()> {
    let home = dirs_next::home_dir().ok_or_else(|| anyhow::anyhow!("cannot find home dir"))?;
    let bin_dir = home.join(".local").join("bin");
    std::fs::create_dir_all(&bin_dir)?;

    let wrapper_path = bin_dir.join(name);

    // Find real binary, skipping our own wrapper
    let real_bin = {
        let mut found = None;
        for candidate_name in cmd_candidates {
            let our_wrapper = bin_dir.join(candidate_name);
            let path_var = std::env::var("PATH").unwrap_or_default();
            for dir in path_var.split(':') {
                let candidate = std::path::Path::new(dir).join(candidate_name);
                if candidate.exists() && candidate != our_wrapper {
                    found = Some(candidate.to_string_lossy().to_string());
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        found.unwrap_or_else(|| {
            println!(
                "⚠️  Could not find '{}' in PATH — wrapper will use PATH lookup at runtime.",
                name
            );
            name.to_string()
        })
    };

    let cap_name = {
        let mut c = name.chars();
        match c.next() {
            None => String::new(),
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        }
    };

    let wrapper = format!(
        r#"#!/usr/bin/env bash
# jmux {name} wrapper — auto-generated by jmux setup {name}
# Real binary: {real_bin}

if [ -n "$JMUX_SOCKET" ]; then
    jmux set-status working "{cap_name} running" 2>/dev/null
fi

"{real_bin}" "$@"
EXIT_CODE=$?

if [ -n "$JMUX_SOCKET" ]; then
    if [ $EXIT_CODE -eq 0 ]; then
        jmux set-status idle 2>/dev/null
    else
        jmux set-status error "{cap_name} exited ($EXIT_CODE)" 2>/dev/null
    fi
fi

exit $EXIT_CODE
"#
    );

    std::fs::write(&wrapper_path, &wrapper)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper_path, std::fs::Permissions::from_mode(0o755))?;
    }

    println!("Created {name} wrapper at {}", wrapper_path.display());

    let in_path = std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .any(|p| p == bin_dir.to_string_lossy().as_ref());
    if !in_path {
        println!("⚠️  Add ~/.local/bin to your PATH:");
        println!("   echo 'export PATH=\"$HOME/.local/bin:$PATH\"' >> ~/.zshrc");
    }
    println!("✓ {cap_name} will now update jmux pane borders when running inside a jmux session.");

    Ok(())
}

fn find_real_codex(home: &std::path::Path) -> Result<String> {
    // Look for codex in PATH, skipping our own wrapper location
    let wrapper_location = home.join(".local").join("bin").join("codex");

    let path_var = std::env::var("PATH").unwrap_or_default();
    for dir in path_var.split(':') {
        let candidate = std::path::Path::new(dir).join("codex");
        if candidate.exists() && candidate != wrapper_location {
            return Ok(candidate.to_string_lossy().to_string());
        }
    }

    // Fallback: assume it'll be in PATH at runtime as "codex" (will cause infinite loop if wrapper
    // is first in PATH — warn about this)
    println!("⚠️  Could not find 'codex' binary in PATH. Make sure it's installed before the wrapper runs.");
    Ok("codex".to_string())
}

fn setup_claude() -> Result<()> {
    let home =
        dirs_next::home_dir().ok_or_else(|| anyhow!("could not determine home directory"))?;
    let settings_path = home.join(".claude").join("settings.json");

    // Read existing JSON or start with empty object
    let existing_text =
        std::fs::read_to_string(&settings_path).unwrap_or_else(|_| "{}".to_string());
    let mut root: serde_json::Value =
        serde_json::from_str(&existing_text).unwrap_or_else(|_| serde_json::json!({}));

    // Ensure root is an object
    if !root.is_object() {
        root = serde_json::json!({});
    }

    // Ensure hooks object exists
    if root.get("hooks").is_none() {
        root["hooks"] = serde_json::json!({});
    }

    let hooks_to_add = [
        ("PreToolUse", "jmux set-status working"),
        ("PostToolUse", "jmux set-status working"),
        ("Stop", "jmux set-status idle"),
    ];

    let mut changed = vec![];

    for (event, command) in &hooks_to_add {
        let hooks_arr = root["hooks"][event].as_array().cloned().unwrap_or_default();

        // Replace any existing jmux hook with the current command (handles upgrades)
        let mut new_arr: Vec<serde_json::Value> = hooks_arr
            .into_iter()
            .filter(|entry| {
                // Remove old jmux hooks so we can re-add the updated one
                if let Some(inner_hooks) = entry.get("hooks").and_then(|h| h.as_array()) {
                    !inner_hooks.iter().any(|h| {
                        h.get("command")
                            .and_then(|c| c.as_str())
                            .map(|c| c.contains("jmux set-status"))
                            .unwrap_or(false)
                    })
                } else {
                    true
                }
            })
            .collect();

        new_arr.push(serde_json::json!({
            "matcher": "",
            "hooks": [{"type": "command", "command": command}]
        }));
        root["hooks"][event] = serde_json::Value::Array(new_arr);
        changed.push(*event);
    }

    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&root)?;
    std::fs::write(&settings_path, json)?;
    println!(
        "Updated {} with jmux hooks for: {}",
        settings_path.display(),
        changed.join(", ")
    );

    Ok(())
}
