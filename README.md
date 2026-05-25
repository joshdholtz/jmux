# jmux

Terminal multiplexer built for developers running AI coding agents.

jmux works like tmux or GNU Screen — split your terminal into panes, manage sessions, detach and reattach — but it also watches what your agents are doing and shows live state on every pane border. When Claude Code is thinking, the border animates. When it needs input, it pulses yellow. When it errors, it turns red. You can run a dozen agents across a dozen projects and see who needs attention at a glance.

```
┌─ my-app ──────────────────────────────────────────────────┐
│  ⠋ [1] agent        ● [2] server        ○ [3] shell       │
╠══════════════════════════════════╦════════════════════════╣
║ > claude                         ║ > cargo run            ║
║ Analyzing your codebase...       ║ Compiling my-app v0.1  ║
║ ...                              ║ ...                    ║
╚══════════════════════════════════╩════════════════════════╝
  [working]                          [idle]
```

---

## Features

- **Agent state indicators** — pane borders animate to show working / waiting / error states for Claude Code, Codex, and any custom agent
- **GNU Screen keybindings** — `ctrl-a` prefix, muscle memory transfers immediately
- **Daemon mode** — detach and reattach without killing your shells or agents
- **Project awareness** — auto-detects git root, project name, and restores pane layouts per project
- **`.jmux.toml` config** — define panes and startup commands per project
- **Fuzzy project picker** — `ctrl-a f` to jump between projects
- **Shell integration** — `chpwd` hook keeps pane directories in sync; `j` alias for quick navigation
- **Narrow mode** — automatically switches to a compact layout when terminal width drops below 100 columns

---

## Installation

### Homebrew (macOS)

```sh
brew install --HEAD joshholtz/jmux/jmux
```

Or with the local formula:

```sh
brew install --formula Formula/jmux.rb
```

### Cargo

```sh
cargo install --path .
```

Requires Rust 1.70+ (2021 edition).

### Shell integration (recommended)

Add to `~/.zshrc`:

```sh
eval "$(jmux init zsh)"
```

Or for bash, add to `~/.bashrc`:

```sh
eval "$(jmux init bash)"
```

This adds a `chpwd` hook so jmux tracks directory changes inside panes, and installs a `j` alias for quick session switching.

---

## Quick start

```sh
cd ~/projects/my-app
jmux
```

That's it. jmux detects the git root, names the session after the project, and opens a shell pane. If a `.jmux.toml` exists in the project root, it uses that instead.

If a daemon is already running for this project, `jmux` attaches to it automatically.

---

## Agent setup

This is the main reason jmux exists. Run one of these once per machine:

### Claude Code

```sh
jmux setup claude
```

Merges three hooks into `~/.claude/settings.json`:

- `PreToolUse` — marks the pane as working
- `PostToolUse` — marks the pane as idle
- `Stop` — marks the pane as waiting for input

The hooks are safe to add alongside existing hooks. If jmux hooks are already present, the command skips them.

### Codex

```sh
jmux setup codex
```

Installs a transparent wrapper at `~/.local/bin/codex` that reports state to jmux before and after each run. The wrapper is a no-op outside a jmux session, so it doesn't affect your normal workflow.

Make sure `~/.local/bin` is early in your `PATH`:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

### Agent state indicators

| Border style | Meaning |
|---|---|
| Plain white | Idle — agent not running or waiting |
| Animated blue double border + braille spinner (⠋) | Working — agent is actively running |
| Pulsing yellow/amber border + ⚡ | Waiting — agent needs your input |
| Red double border + ✗ | Error — agent exited with a failure |

---

## Keybindings

All keybindings use the `ctrl-a` prefix (GNU Screen style).

| Key | Action |
|---|---|
| `ctrl-a n` | Next pane |
| `ctrl-a p` | Previous pane |
| `ctrl-a N` | Next session |
| `ctrl-a P` | Previous session |
| `ctrl-a "` or `ctrl-a %` | New pane |
| `ctrl-a x` | Close active pane |
| `ctrl-a ,` | Rename active pane |
| `ctrl-a z` | Zoom / unzoom active pane |
| `ctrl-a [` or `PageUp` | Enter scrollback / copy mode |
| `ctrl-a 1`–`9` | Jump to pane by number |
| `ctrl-a d` | Detach TUI (daemon keeps running); or show dashboard in standalone mode |
| `ctrl-a f` | Fuzzy project picker |
| `ctrl-a q` | Quit |

### Scrollback mode

Enter with `ctrl-a [` or `PageUp`. Exit with `q`.

| Key | Action |
|---|---|
| `j` / `k` | Scroll down / up one line |
| `ctrl-d` / `ctrl-u` | Scroll down / up half page |
| `g` | Jump to top |
| `G` | Jump to bottom |
| `q` | Exit scrollback mode |

---

## Daemon mode

Run jmux as a background daemon so your shells and agents keep running after you close the terminal.

### Start the daemon

```sh
jmux daemon
```

The daemon owns all PTYs for the current project. Its socket is at `/tmp/jmux-daemon-{hash}.sock`, where the hash is derived from the project root path — so each project gets its own daemon.

### Attach

```sh
jmux
```

If a daemon is running for the current project directory, `jmux` connects to it automatically. No flags needed.

### Detach

Press `ctrl-a d` inside a jmux session. The TUI exits; the daemon and all shells continue running in the background.

### Reattach

```sh
jmux
# or from anywhere:
jmux attach
```

---

## Session management

```sh
jmux ls                  # list saved sessions
jmux new [path]          # open a new session for a path (defaults to cwd)
jmux kill <name>         # remove a saved session
```

`jmux ls` shows session names, project roots, pane counts, and last-known status.

If you use the `j` alias from shell integration:

```sh
j                        # attach to jmux (or start it)
j ~/projects/other-app   # open or switch to that project
```

---

## Config file

Place `.jmux.toml` in your project root to define a fixed pane layout and startup commands.

```toml
[project]
name = "my-app"

[[panes]]
name = "server"
cmd = "cargo run"

[[panes]]
name = "agent"
cmd = "claude"

[[panes]]
name = "shell"
```

- `project.name` overrides the auto-detected project name in the status bar.
- `panes[].cmd` runs automatically when the pane opens. Omit it for a plain shell.
- Panes are ordered top-to-bottom / left-to-right depending on terminal width.

---

## Socket protocol

Every pane has `$JMUX_SOCKET` set in its environment. Agents and scripts can use this to push state updates directly.

### CLI

```sh
jmux set-status working "Running tests"
jmux set-status idle
jmux set-status waiting "needs your input"
jmux set-status error "build failed"
jmux flash
```

`jmux set-status` and `jmux flash` are no-ops outside a jmux session (when `$JMUX_SOCKET` is not set), so they're safe to use in scripts unconditionally.

### Raw socket

Messages are newline-delimited JSON sent to the Unix socket at `$JMUX_SOCKET`.

```json
{"method": "set-status", "params": {"state": "working", "message": "Running tests"}}
{"method": "set-status", "params": {"state": "idle"}}
{"method": "set-status", "params": {"state": "waiting", "message": "needs input"}}
{"method": "set-status", "params": {"state": "error", "message": "build failed"}}
{"method": "flash", "params": {}}
```

Valid states: `working`, `idle`, `waiting`, `error`.

Use the raw socket to integrate jmux state reporting into any tool that can write to a Unix socket — CI scripts, build systems, custom agents.

---

## Narrow / mobile mode

When the terminal width drops below 100 columns, jmux automatically switches to a compact layout:

- Top bar shrinks to a single line showing the project name and a row of pane dots
- Each dot reflects the pane's agent state (color-coded)
- Panes stack vertically instead of side by side

This makes jmux usable on a laptop with a narrow terminal or split alongside an editor.

---

## Building from source

```sh
git clone https://github.com/joshholtz/jmux
cd jmux
cargo build --release
# binary is at target/release/jmux
```

### Workspace layout

| Crate | Contents |
|---|---|
| `src/` | CLI entry point — argument parsing, subcommand dispatch |
| `crates/jmux-core/` | State types, project detection, config parsing, persistence |
| `crates/jmux-tui/` | TUI renderer, PTY management, socket server, daemon |

### Key dependencies

- [`ratatui`](https://github.com/ratatui-org/ratatui) — TUI rendering
- [`portable-pty`](https://github.com/wez/wezterm/tree/main/pty) — cross-platform PTY multiplexing
- [`vt100`](https://github.com/doy/vt100-rust) — terminal emulation / screen buffer
- [`tokio`](https://tokio.rs) — async runtime for the socket server and daemon
- [`crossterm`](https://github.com/crossterm-rs/crossterm) — terminal input and raw mode

### State persistence

Session layout is saved to `~/.local/share/jmux/state.json` on exit. The daemon socket lives at `/tmp/jmux-daemon-{hash}.sock` where `{hash}` is a stable 64-bit hash of the project root path.

---

## License

MIT
