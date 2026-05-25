# jmux — LLM context

Terminal multiplexer for people who run a lot of things at once. Rust workspace, tokio async, always-daemon architecture.

## Commands you'll actually need

```sh
cargo build                   # fast build (no install)
cargo install --path .        # install jmux binary to ~/.cargo/bin
cargo test -p jmux-core       # run unit tests (jmux-core only; TUI tests require a PTY)
cargo fmt                     # format
cargo clippy -- -D warnings   # lint
```

After `cargo install`, kill the running daemon so the new binary takes effect:
```sh
pkill -f "jmux daemon" 2>/dev/null; jmux
```
Or just run `jmux` — it detects if the binary is newer than the daemon socket and auto-restarts.

## Workspace layout

```
src/main.rs                    CLI entry point — subcommand dispatch, run_tui()
crates/jmux-core/src/
  lib.rs                       re-exports; AgentState, AppState, Session, PaneState
  state.rs                     core state types (serializable)
  project.rs                   git-root detection, ProjectInfo
  config.rs                    .jmux.toml parsing
  persistence.rs               ~/.local/share/jmux/state.json save/load
crates/jmux-tui/src/
  app.rs                       App struct, main event loop, all keybindings
  daemon.rs                    run_daemon() — PTY owner, socket server, broadcast
  daemon_client.rs             DaemonClient — connects to daemon, subscribe/send
  pty.rs                       PtyPane, poll_fg_process, parse_osc_state, detect_agent_state
  socket.rs                    SocketEvent enum for standalone mode
  layout.rs                    layout detection (wide vs narrow)
  input.rs                     key_to_bytes, key event helpers
  widgets/
    session_list.rs            sidebar renderer
    pane_view.rs               PTY screen renderer, dim_color for inactive panes
```

## Architecture: always-daemon

`jmux` (no subcommand) always spawns a background `jmux daemon` subprocess, then immediately connects as a client. There is no standalone mode in normal operation.

```
jmux (client TUI)  ←→  Unix socket  ←→  jmux daemon (owns all PTYs)
```

- **Daemon** (`daemon.rs`): owns PTYs, runs a PTY output loop, handles JSON-RPC over Unix socket, broadcasts state changes to all connected clients.
- **Client** (`app.rs` in daemon mode): renders using `client_ptys` (NullMaster panes fed by PTY broadcast messages), sends keyboard input to daemon via socket.
- **Socket path**: `/tmp/jmux-daemon-{hash}.sock` where hash is derived from the project root path.
- **Auto-restart**: if the binary mtime is newer than the socket mtime, the client kills the old daemon and spawns a fresh one.

## Daemon socket protocol

Newline-delimited JSON. Client sends requests; daemon broadcasts responses to all subscribers.

### Client → Daemon
```json
{"method": "subscribe", "params": {}}
{"method": "input", "params": {"session_id": 0, "pane_id": 1, "data": [107,106]}}
{"method": "resize-pane", "params": {"session_id": 0, "pane_id": 1, "rows": 40, "cols": 120}}
{"method": "resize", "params": {"rows": 40, "cols": 120}}
{"method": "set-status", "params": {"state": "working", "session_id": 0, "pane_id": 1}}
{"method": "focus-pane", "params": {"session_id": 0, "pane_id": 1}}
{"method": "new-session", "params": {"path": "/Users/josh/projects/foo"}}
{"method": "add-pane", "params": {"session_id": 0, "cwd": "/Users/josh/projects/foo"}}
{"method": "close-pane", "params": {"session_id": 0, "pane_id": 1}}
{"method": "kill-session", "params": {"name": "jmux"}}
{"method": "set-name", "params": {"name": "cargo", "session_id": 0, "pane_id": 1}}
{"method": "set-cwd", "params": {"path": "/Users/josh/projects/foo"}}
{"method": "detach", "params": {}}
```

### Daemon → Client (broadcast)
```json
{"type": "state", "data": <AppState>}
{"type": "pty", "session_id": 0, "pane_id": 1, "data": [27,91,...]}
```

## Agent state

`AgentState` (in `jmux-core/src/state.rs`) has four variants: `Idle`, `Working { message }`, `Waiting { message }`, `Error { message }`.

How state gets set:
1. **Claude Code hooks** — `PreToolUse`/`PostToolUse` → `jmux set-status working`, `Stop` → `jmux set-status idle`. Configured by `jmux setup claude`.
2. **OSC sequences** — any tool can emit `\e]9999;state=working;message=doing stuff\a` to set state without needing `JMUX_SOCKET`.
3. **Text pattern detection** — `detect_agent_state()` in `pty.rs` matches output from Codex, Aider, Gemini, Copilot CLI.
4. **Process poll** — daemon polls `process_group_leader()` every 500ms; clears `Working`/`Waiting` when fg process returns to shell.
5. **Input received** — any input to a `Waiting` pane clears it to `Idle`.
6. **Focus change** — switching to a `Waiting` pane sends `focus-pane` which clears it to `Idle`.

## Pane name resolution (sidebar)

Priority (highest to lowest):
1. `pane.process_name` — set by input buffer tracking (daemon buffers keystrokes, extracts first word on Enter when shell is fg) or by `poll_fg_process()` when a non-shell process takes over
2. `pane.name` — set at pane creation (shell binary name) or by `ctrl-a ,` rename

## Environment variables set on pane processes

```
JMUX_SOCKET       path to daemon Unix socket
JMUX_SESSION_ID   session ID (usize)
JMUX_PANE_ID      pane ID (usize)
TERM              xterm-256color
COLORTERM         truecolor
```

Hooks and scripts use `JMUX_SESSION_ID`/`JMUX_PANE_ID` to target the correct pane when calling `jmux set-status`.

## Key keybindings (prefix = ctrl-a)

```
n/p          next/prev pane
N/P          next/prev session
" or %       new pane
x            close pane
,            rename pane
z            zoom pane
[  PageUp    enter scrollback/copy mode
d            detach
f            fuzzy project picker
q            quit
1-9          jump to pane by number
```

## Shell integration

`jmux init zsh` / `jmux init bash` prints a shell script that:
- Adds `chpwd` hook → `jmux set-cwd $PWD`
- Adds `preexec` hook → `jmux set-name <first-word-of-command>`
- Defines `j` alias

## Things to know

- **clippy warnings** — there are two dead-code warnings for `run_tui_standalone` and `to_saved_state` in `src/main.rs`. These are kept for potential standalone mode revival. Don't delete them.
- **PTY sizing** — client computes actual pane rect sizes from terminal dimensions and sends `resize-pane` to daemon. Daemon owns the real PTY, client has NullMaster.
- **Broadcast is lossy** — `broadcast::channel` drops messages if the receiver is slow. This is intentional; PTY output is best-effort.
- **Scrollback** — stored in `PtyPane.scrollback` (VecDeque<String>, max 5000 lines) as stripped plain text. The `vt100::Parser` holds the rendered screen buffer.
