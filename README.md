# jmux

Terminal multiplexer for people who run a lot of things at once.

Split your terminal into panes, manage sessions across projects, detach and reattach — and always know what everything is doing. Each pane shows what's running and whether it needs your attention. Switch between a build, a server, a REPL, and a few AI agents without losing track of any of them.

```
╔═ jmux ═══════════════════════════════════════════════════════════════════════╗
║ Sessions              ║                                                      ║
║ ▶ jmux                ║  > cargo build --release                             ║
║   ▸ cargo ⣾           ║  Compiling jmux v0.1.0                               ║
║     vim ·             ║  ...                                                  ║
║     claude ⚡          ╠══════════════════════════════════════════════════════╣
║                       ║  > vim src/main.rs                                   ║
║ ● my-app              ║                                                       ║
║   ▸ server ·          ║                                                       ║
║     tests ·           ║                                                       ║
╚═══════════════════════╩══════════════════════════════════════════════════════╝
```

---

## Features

- **See what's running** — the sidebar shows every pane across every session, with the current command and whether it's busy, waiting, or idle
- **Always-daemon architecture** — detach and reattach without killing anything; your shells, servers, and agents keep running
- **Multi-project sessions** — each project gets its own session; jump between them instantly
- **GNU Screen keybindings** — `ctrl-a` prefix; no new muscle memory required
- **Project awareness** — auto-detects git root, names sessions by project, restores pane layouts
- **`.jmux.toml` config** — define panes and startup commands per project
- **Fuzzy project picker** — `ctrl-a f` to jump between projects
- **Agent state indicators** — if you run AI agents (Claude Code, Codex, Aider, Gemini), their pane borders show working/waiting/error state so you know when they need input without switching to them
- **Dimmed inactive panes** — unfocused panes render at reduced intensity so your eyes go to the right place

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

Split a pane: `ctrl-a "`. Close it: `ctrl-a x`. Detach: `ctrl-a d`.

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

Place `.jmux.toml` in your project root to define a fixed pane layout:

```toml
[project]
name = "my-app"

[[panes]]
name = "server"
cmd = "cargo run"

[[panes]]
name = "tests"
cmd = "cargo watch -x test"

[[panes]]
name = "shell"
```

Panes without `cmd` open a plain shell.

---

## AI agent integration

If you use AI coding agents, jmux can show you their state in the sidebar without switching to their pane.

### Claude Code

```sh
jmux setup claude
```

Adds hooks to `~/.claude/settings.json` that mark the pane as working (⣾) when Claude is running a tool, and idle (·) when it stops.

### Codex / Aider / Gemini

```sh
jmux setup codex
jmux setup aider
jmux setup gemini
jmux setup all
```

Installs a transparent wrapper that reports state to jmux. No-op outside a jmux session.

### Any tool — OSC sequence

```sh
printf '\e]9999;state=working;message=running tests\a'
printf '\e]9999;state=idle\a'
```

### Agent state indicators

| Icon | Meaning |
|------|---------|
| `·` | Idle |
| `⣾` | Working |
| `⚡` | Waiting for your input |
| `✗` | Error |

---

## Building from source

```sh
git clone https://github.com/joshdholtz/jmux
cd jmux
cargo build --release
```

### Dev workflow

```sh
cargo build                  # compile
cargo test -p jmux-core      # run tests
cargo fmt                    # format
cargo clippy -- -D warnings  # lint
```

Git hooks install automatically the first time you run `cargo test`. The pre-push hook runs `cargo fmt --check` and `cargo clippy`.

---

## License

MIT
