use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::App;
use crate::layout::LayoutMode;

pub fn render_status_bar(f: &mut Frame, area: Rect, app: &App) {
    match app.layout_mode {
        LayoutMode::Wide => render_wide_status(f, area, app),
        LayoutMode::Narrow => render_narrow_status(f, area, app),
    }
}

fn render_wide_status(f: &mut Frame, area: Rect, app: &App) {
    let session = match app.state.sessions.get(app.state.active_session) {
        Some(s) => s,
        None => return,
    };

    let project_name = session
        .project
        .as_ref()
        .map(|p| p.name.as_str())
        .unwrap_or("no project");

    let active_pane_cwd = session
        .panes
        .get(session.active_pane)
        .map(|p| p.cwd.clone());

    let cwd_display = active_pane_cwd
        .as_ref()
        .and_then(|cwd| {
            let root = session.project.as_ref().map(|p| &p.root)?;
            cwd.strip_prefix(root).ok().map(|rel| {
                let s = rel.to_string_lossy();
                if s.is_empty() {
                    "~".to_string()
                } else {
                    format!("~/{}", s)
                }
            })
        })
        .unwrap_or_else(|| {
            active_pane_cwd
                .as_ref()
                .map(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string()
                })
                .unwrap_or_default()
        });

    let left = match app.git_branch.as_deref() {
        Some(branch) if !branch.is_empty() => {
            if cwd_display.is_empty() {
                format!(" {} · {} ", project_name, branch)
            } else {
                format!(" {} · {} · {} ", project_name, branch, cwd_display)
            }
        }
        _ => {
            if cwd_display.is_empty() {
                format!(" {} ", project_name)
            } else {
                format!(" {} · {} ", project_name, cwd_display)
            }
        }
    };

    let center = if app.is_daemon_mode() {
        "⇆ DAEMON  ^Ad:detach  n/p:session  N/P:pane"
    } else {
        "^Ad:detach  n/p:pane  N/P:session  \":split  x:close"
    };
    let attention = app.state.attention_count();

    let time_str = chrono::Local::now().format("%H:%M").to_string();

    let bar_style = Style::default().bg(Color::DarkGray).fg(Color::White);

    let right_line = if attention > 0 {
        Line::from(vec![
            Span::styled(
                format!(" ⚡{} ", attention),
                Style::default()
                    .bg(Color::DarkGray)
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {} ", time_str),
                Style::default().bg(Color::DarkGray).fg(Color::DarkGray),
            ),
        ])
    } else {
        Line::from(Span::styled(
            format!(" {} ", time_str),
            Style::default().bg(Color::DarkGray).fg(Color::DarkGray),
        ))
    };

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(50),
            Constraint::Percentage(25),
        ])
        .split(area);

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(left, bar_style))),
        chunks[0],
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(center, bar_style))).centered(),
        chunks[1],
    );
    f.render_widget(Paragraph::new(right_line).right_aligned(), chunks[2]);
}

fn render_narrow_status(f: &mut Frame, area: Rect, app: &App) {
    let session = match app.state.sessions.get(app.state.active_session) {
        Some(s) => s,
        None => return,
    };

    let project_name = session
        .project
        .as_ref()
        .map(|p| p.name.as_str())
        .unwrap_or("no project");

    let pane_count = session.panes.len();
    let active_pane = session.active_pane + 1;

    let cwd_str = session
        .panes
        .get(session.active_pane)
        .and_then(|p| p.cwd.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("");

    let text = if cwd_str.is_empty() {
        format!(
            " {} — {}/{} panes  ^Ad:detach n/p:pane N/P:session",
            project_name, active_pane, pane_count
        )
    } else {
        format!(
            " {} · {} — {}/{}  ^Ad:detach n/p:pane N/P:session",
            project_name, cwd_str, active_pane, pane_count
        )
    };

    f.render_widget(
        Paragraph::new(text).style(Style::default().bg(Color::DarkGray).fg(Color::White)),
        area,
    );
}
