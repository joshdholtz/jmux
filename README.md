# jmux

Terminal multiplexer built for AI agent workflows.

jmux works like tmux — split your terminal into panes, manage sessions, detach and reattach — but it also watches what your agents are doing and shows live status on every pane border. When Claude Code is thinking, the border pulses. When it needs input, it flashes yellow. You can run multiple agents across multiple projects and see who needs attention at a glance.

```
╔═ jmux [working ⣾] ══════════════════════════════════════════════════════════╗
║ Sessions              ║                                                      ║
║ ▶ jmux                ║  > claude                                            ║
║   ▸ claude ⣾          ║  Analyzing your codebase...                          ║
║     shell ·           ║                                                      ║
║                       ╠══════════════════════════════════════════════════════╣
║                       ║  > cargo build                                       ║
║                       ║  Compiling jmux v0.1.0                               ║
╚═══════════════════════╩══════════════════════════════════════════════════════╝
```

---

## Features

- **Agent state indicators** — pane borders animate to show working/waiting/error for Claude Code, Codex, Aider, Gemini, and any custom tool
- **Always-daemon architecture** — `jmux` auto-spawns a background daemon and attaches; detach and reattach without killing shells or agents
- **GNU Screen keybindings** — `ctrl-a` prefix; muscle memory transfers immediately
- **Project awareness** — auto-detects git root, names sessions by project, restores pane layouts
- **`.jmux.toml` config** — define panes and startup commands per project
- **Fuzzy project picker** — `ctrl-a f` to jump between projects
- **Shell integration** — `chpwd` and `preexec` hooks keep pane state in sync
- **Sidebar** — shows all sessions and panes with live agent state icons
- **Dimmed inactive panes** — unfocused panes render at reduced intensity

---

## Installation

### Cargo (from source)

```sh
git clone https://github.com/joshdholtz/jmux
cd jmux
cargo install --path .
```

Requires Rust 1.70+.

### Shell integration

Add to `~/.zshrc`:
```sh
eval "$(jmux init zsh)"
```

Or for bash, add to `~/.bashrc`:
```sh
eval "$(jmux init bash)"
```

This adds hooks so jmux tracks directory changes and running commands inside panes, and installs a `j` alias for quick session switching.

---

## Quick start

```sh
cd ~/projects/my-app
jmux
```

jmux detects the git root, names the session after the project, and opens a shell pane. If a daemon is already running for this project, `jmux` attaches to it automatically.

---

## Agent setup

### Claude Code

```sh
jmux setup claude
```

Merges hooks into `~/.claude/settings.json`:

- `PreToolUse` → marks the pane as **working**
- `PostToolUse` → marks the pane as **working**
- `Stop` → marks the pane as **idle**

Safe to run multiple times — existing jmux hooks are replaced, other hooks are left alone.

### Codex / Aider / Gemini

```sh
jmux setup codex
jmux setup aider
jmux setup gemini
jmux setup all       # set up every tool found in PATH
```

Installs a transparent wrapper at `~/.local/bin/<tool>` that reports working/idle state to jmux. The wrapper is a no-op outside a jmux session.

Make sure `~/.local/bin` is early in your `PATH`:
```sh
export PATH="$HOME/.local/bin:$PATH"
```

### Custom tool — OSC sequence

Any tool can report state without needing `JMUX_SOCKET`:

```sh
printf '\e]9999;state=working;message=running tests\a'
printf '\e]9999;state=idle\a'
printf '\e]9999;state=waiting;message=needs approval\a'
printf '\e]9999;state=error;message=build failed\a'
```

### Custom tool — CLI

```sh
jmux set-status working "running tests"
jmux set-status idle
jmux set-status waiting "needs your input"
jmux set-status error "build failed"
```

These are no-ops when `JMUX_SOCKET` is not set (i.e. outside a jmux session).

---

## Agent state indicators

| Icon | Border | Meaning |
|------|--------|---------|
| `·` | normal | Idle |
| `⣾` | animated blue double | Working |
| `⚡` | yellow | Waiting for input |
| `✗` | red double | Error |

---

## Keybindings

All use the `ctrl-a` prefix (GNU Screen style).

| Key | Action |
|-----|--------|
| `ctrl-a n` | Next pane |
| `ctrl-a p` | Previous pane |
| `ctrl-a N` | Next session |
| `ctrl-a P` | Previous session |
| `ctrl-a "` or `ctrl-a %` | New pane |
| `ctrl-a x` | Close active pane |
| `ctrl-a ,` | Rename active pane |
| `ctrl-a z` | Zoom / unzoom active pane |
| `ctrl-a [` or `PageUp` | Enter scrollback mode |
| `ctrl-a 1`–`9` | Jump to pane by number |
| `ctrl-a d` | Detach (daemon keeps running) |
| `ctrl-a f` | Fuzzy project picker |
| `ctrl-a q` | Quit |

### Scrollback mode

Enter with `ctrl-a [` or `PageUp`. Exit with `q` or `Esc`.

| Key | Action |
|-----|--------|
| `j` / `k` | Scroll down / up |
| `ctrl-d` / `ctrl-u` | Half-page down / up |
| `g` / `G` | Top / bottom |

---

## Session management

```sh
jmux ls                # list saved sessions
jmux new [path]        # open a new session for a path (defaults to cwd)
jmux kill <name>       # kill a session
```

---

## Config file

Place `.jmux.toml` in your project root:

```toml
[project]
name = "my-app"

[[panes]]
name = "agent"
cmd = "claude"

[[panes]]
name = "server"
cmd = "cargo run"

[[panes]]
name = "shell"
```

Panes without `cmd` open a plain shell.

---

## Building from source

```sh
git clone https://github.com/joshdholtz/jmux
cd jmux
cargo build --release
# binary: target/release/jmux
```

### Dev workflow

```sh
cargo build                  # compile
cargo test -p jmux-core      # run tests
cargo fmt                    # format
cargo clippy -- -D warnings  # lint
```

Git hooks install automatically the first time you run `cargo test` (via `cargo-husky`). The pre-push hook runs `cargo fmt --check` and `cargo clippy`.

### Workspace

| Crate | Contents |
|-------|----------|
| `src/` | CLI entry point — arg parsing, subcommand dispatch |
| `crates/jmux-core/` | State types, project detection, config, persistence |
| `crates/jmux-tui/` | TUI renderer, PTY management, daemon, socket client |

---

## License

MIT
