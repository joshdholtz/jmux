use std::time::{SystemTime, UNIX_EPOCH};

use jmux_core::{AgentState, PaneState};
use ratatui::{
    layout::Rect,
    prelude::Stylize,
    style::{Color, Modifier, Style},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

fn dim_color(color: Color) -> Color {
    match color {
        Color::Rgb(r, g, b) => Color::Rgb(r / 2, g / 2, b / 2),
        Color::Reset => Color::Indexed(238), // dark gray
        Color::White => Color::Indexed(240),
        Color::Gray => Color::Indexed(236),
        Color::DarkGray => Color::Indexed(234),
        Color::Red => Color::Indexed(88),
        Color::Green => Color::Indexed(22),
        Color::Yellow => Color::Indexed(58),
        Color::Blue => Color::Indexed(18),
        Color::Magenta => Color::Indexed(54),
        Color::Cyan => Color::Indexed(23),
        Color::LightRed => Color::Red,
        Color::LightGreen => Color::Green,
        Color::LightBlue => Color::Blue,
        Color::LightCyan => Color::Cyan,
        Color::LightMagenta => Color::Magenta,
        Color::LightYellow => Color::Yellow,
        Color::Black => Color::Black,
        Color::Indexed(n) => {
            if n >= 8 && n <= 15 {
                // bright → normal
                Color::Indexed(n - 8)
            } else if n >= 16 && n <= 231 {
                // xterm 6x6x6 RGB cube: halve each component
                let i = n - 16;
                let r = i / 36;
                let g = (i / 6) % 6;
                let b = i % 6;
                Color::Indexed(16 + (r / 2) * 36 + (g / 2) * 6 + b / 2)
            } else if n >= 232 {
                // grayscale ramp: step down by ~6 levels
                Color::Indexed(232 + (n - 232).saturating_sub(6) / 2)
            } else {
                color
            }
        }
    }
}

use crate::pty::PtyPane;

pub fn render_pane(
    f: &mut Frame,
    area: Rect,
    pane: &PaneState,
    pty: Option<&PtyPane>,
    is_active: bool,
) {
    render_pane_inner(f, area, pane, pty, is_active, false)
}

pub fn render_pane_daemon(
    f: &mut Frame,
    area: Rect,
    pane: &PaneState,
    pty: Option<&PtyPane>,
    is_active: bool,
) {
    render_pane_inner(f, area, pane, pty, is_active, true)
}

fn render_pane_inner(
    f: &mut Frame,
    area: Rect,
    pane: &PaneState,
    pty: Option<&PtyPane>,
    is_active: bool,
    daemon_mode: bool,
) {
    let now_millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_millis();

    let pulse = (now_millis / 500).is_multiple_of(2);

    let (border_style, border_type, state_indicator, extra_title) = match &pane.agent_state {
        AgentState::Idle => (
            Style::default().fg(if is_active {
                Color::White
            } else {
                Color::DarkGray
            }),
            BorderType::Plain,
            "✓",
            "",
        ),
        AgentState::Working { .. } => {
            let spinner_frames = ["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"];
            let frame_idx = (now_millis / 125) as usize % spinner_frames.len();
            let spinner = spinner_frames[frame_idx];
            (
                Style::default().fg(Color::Blue),
                BorderType::Double,
                spinner,
                "",
            )
        }
        AgentState::Waiting { .. } => {
            if pulse {
                (
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                    BorderType::Double,
                    "⚡",
                    " ⚡ WAITING",
                )
            } else {
                (
                    Style::default().fg(Color::Rgb(180, 120, 0)),
                    BorderType::Thick,
                    "!",
                    "  ! WAITING",
                )
            }
        }
        AgentState::Error { .. } => (
            Style::default().fg(Color::Red),
            BorderType::Double,
            "✗",
            " ✗ ERROR",
        ),
    };

    let title = format!(" {} {}{} ", state_indicator, pane.name, extra_title);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(border_style)
        .title(title);

    let inner = block.inner(area);
    f.render_widget(block, area);

    if let Some(pty) = pty {
        render_pty_screen(f, inner, pty, is_active);
    } else {
        // Placeholder text when no PTY is available
        use ratatui::text::{Line, Text};
        let mut lines: Vec<Line> = vec![];

        if let Some(proc) = &pane.process_name {
            lines.push(Line::from(format!("process: {}", proc)));
        }

        let message = match &pane.agent_state {
            AgentState::Working { message: Some(m) } => Some(m.as_str()),
            AgentState::Waiting { message: Some(m) } => Some(m.as_str()),
            AgentState::Error { message: Some(m) } => Some(m.as_str()),
            _ => None,
        };

        if let Some(msg) = message {
            lines.push(Line::from(format!("  {}", msg)));
        }

        let placeholder = if daemon_mode {
            "⇆ connected to daemon · ^A D to detach"
        } else {
            "(pty output will render here)"
        };
        lines.push(Line::from(ratatui::text::Span::styled(
            placeholder,
            Style::default().fg(Color::DarkGray),
        )));

        f.render_widget(Paragraph::new(Text::from(lines)), inner);
    }
}

pub fn render_pane_content(f: &mut Frame, area: Rect, pane: &PaneState, pty: Option<&PtyPane>) {
    if let Some(pty) = pty {
        render_pty_screen(f, area, pty, true);
    } else {
        let msg = match &pane.agent_state {
            AgentState::Working { message: Some(m) } => format!("⣾ {}", m),
            AgentState::Waiting { message: Some(m) } => format!("⚡ {}", m),
            AgentState::Error { message: Some(m) } => format!("✗ {}", m),
            _ => "(no pty output)".to_string(),
        };
        f.render_widget(
            Paragraph::new(msg).style(Style::default().fg(Color::DarkGray)),
            area,
        );
    }
}

pub fn render_scrollback(f: &mut Frame, area: Rect, pty: &PtyPane, scroll_offset: usize) {
    let lines = pty.scrollback_lines();
    let total = lines.len();
    let visible = area.height as usize;

    // scroll_offset lines from bottom: show lines[total - visible - scroll_offset .. total - scroll_offset]
    let end = total.saturating_sub(scroll_offset);
    let start = end.saturating_sub(visible);

    let mut display_lines: Vec<ratatui::text::Line> = lines
        .iter()
        .skip(start)
        .take(visible)
        .map(|l| ratatui::text::Line::from(l.as_str()))
        .collect();

    // Pad to fill area
    while display_lines.len() < visible {
        display_lines.push(ratatui::text::Line::from(""));
    }

    let scroll_indicator = if scroll_offset > 0 {
        format!(
            " SCROLL [{} lines up] (j/k scroll, g/G top/bottom, q exit) ",
            scroll_offset
        )
    } else {
        " SCROLL MODE (press q or esc to exit) ".to_string()
    };

    // Render scrollback content
    f.render_widget(Paragraph::new(display_lines), area);

    // Render a status line at the top of the area (1 line overlay)
    let status_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    f.render_widget(
        Paragraph::new(scroll_indicator)
            .style(Style::default().bg(Color::DarkGray).fg(Color::Yellow)),
        status_area,
    );
}

fn render_pty_screen(f: &mut Frame, area: Rect, pty: &PtyPane, is_active: bool) {
    let screen = pty.parser.screen();
    let rows = area.height as usize;
    let cols = area.width as usize;

    let lines: Vec<ratatui::text::Line> = (0..rows)
        .map(|row| {
            let mut spans = Vec::new();
            let mut current_style = ratatui::style::Style::default();
            let mut current_content = String::new();

            for col in 0..cols {
                let (content, style) = if let Some(cell) = screen.cell(row as u16, col as u16) {
                    let fg = vt100_color(cell.fgcolor());
                    let bg = vt100_color(cell.bgcolor());
                    let fg = if is_active { fg } else { dim_color(fg) };
                    let mut s = ratatui::style::Style::default().fg(fg).bg(bg);
                    if cell.bold() && is_active {
                        s = s.bold();
                    }
                    if cell.italic() {
                        s = s.italic();
                    }
                    if cell.underline() {
                        s = s.underlined();
                    }
                    let c = cell.contents();
                    (if c.is_empty() { " ".to_string() } else { c }, s)
                } else {
                    (" ".to_string(), ratatui::style::Style::default())
                };

                if style != current_style && !current_content.is_empty() {
                    spans.push(ratatui::text::Span::styled(
                        current_content.clone(),
                        current_style,
                    ));
                    current_content.clear();
                }
                current_style = style;
                current_content.push_str(&content);
            }

            if !current_content.is_empty() {
                spans.push(ratatui::text::Span::styled(current_content, current_style));
            }

            ratatui::text::Line::from(spans)
        })
        .collect();

    let paragraph = ratatui::widgets::Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

fn vt100_color(color: vt100::Color) -> ratatui::style::Color {
    match color {
        vt100::Color::Default => ratatui::style::Color::Reset,
        vt100::Color::Idx(n) => ratatui::style::Color::Indexed(n),
        vt100::Color::Rgb(r, g, b) => ratatui::style::Color::Rgb(r, g, b),
    }
}
