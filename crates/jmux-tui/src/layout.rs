use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    Frame,
};

use crate::app::{App, RenameTarget};
use crate::widgets::{
    dashboard::render_dashboard,
    pane_view::{render_pane, render_pane_content, render_pane_daemon},
    rename_prompt::render_rename_prompt,
    session_list::render_session_list,
    status_bar::render_status_bar,
};

#[derive(Debug, Clone, PartialEq)]
pub enum LayoutMode {
    Wide,
    Narrow,
}

pub fn detect_layout(width: u16) -> LayoutMode {
    if width >= 100 {
        LayoutMode::Wide
    } else {
        LayoutMode::Narrow
    }
}

pub fn render_wide(f: &mut Frame, app: &App) {
    app.pane_rects.borrow_mut().clear();

    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let main_area = chunks[0];
    let status_area = chunks[1];

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(22), Constraint::Min(0)])
        .split(main_area);

    let sidebar_area = columns[0];
    let panes_area = columns[1];

    render_session_list(f, sidebar_area, app);
    render_status_bar(f, status_area, app);

    let session = match app.state.sessions.get(app.state.active_session) {
        Some(s) => s,
        None => {
            if app.show_dashboard {
                render_dashboard(f, app);
            }
            return;
        }
    };

    if session.panes.is_empty() {
        if app.show_dashboard {
            render_dashboard(f, app);
        }
        return;
    }

    if app.zoomed {
        // Render only the active pane filling the entire panes_area
        if let Some(pane) = session.panes.get(session.active_pane) {
            app.pane_rects
                .borrow_mut()
                .insert((session.id, pane.id), panes_area);
            let is_copy_mode = app
                .copy_mode
                .as_ref()
                .map(|cm| cm.session_id == session.id && cm.pane_id == pane.id)
                .unwrap_or(false);
            if is_copy_mode {
                if let Some(pty) = app.pty_for(session.id, pane.id) {
                    let offset = app.copy_mode.as_ref().unwrap().scroll_offset;
                    use ratatui::widgets::{Block, Borders};
                    let block = Block::default()
                        .borders(Borders::ALL)
                        .border_style(
                            ratatui::style::Style::default().fg(ratatui::style::Color::Yellow),
                        )
                        .title(" COPY MODE -- q/esc exit  k/up up  j/dn down  g top  G bottom ");
                    let inner = block.inner(panes_area);
                    f.render_widget(block, panes_area);
                    crate::widgets::pane_view::render_scrollback(f, inner, pty, offset);
                }
            } else {
                let pty = app.pty_for(session.id, pane.id);
                if app.is_daemon_mode() {
                    render_pane_daemon(f, panes_area, pane, pty, true);
                } else {
                    render_pane(f, panes_area, pane, pty, true);
                }
            }
        }
    } else {
        let pane_count = session.panes.len();
        let rows = if pane_count <= 2 { 1 } else { 2 };
        let cols = pane_count.div_ceil(rows);

        let row_constraints: Vec<Constraint> = (0..rows)
            .map(|_| Constraint::Ratio(1, rows as u32))
            .collect();
        let row_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(row_constraints)
            .split(panes_area);

        let mut pane_idx = 0;
        for row in 0..rows {
            let panes_in_row = if row < pane_count % rows || pane_count % rows == 0 {
                cols
            } else {
                cols - 1
            }
            .min(pane_count - pane_idx);

            let col_constraints: Vec<Constraint> = (0..panes_in_row)
                .map(|_| Constraint::Ratio(1, panes_in_row as u32))
                .collect();
            let col_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(col_constraints)
                .split(row_chunks[row]);

            for col in 0..panes_in_row {
                if let Some(pane) = session.panes.get(pane_idx) {
                    let is_active = pane_idx == session.active_pane;
                    app.pane_rects
                        .borrow_mut()
                        .insert((session.id, pane.id), col_chunks[col]);
                    let is_copy_mode = app
                        .copy_mode
                        .as_ref()
                        .map(|cm| cm.session_id == session.id && cm.pane_id == pane.id)
                        .unwrap_or(false);
                    if is_copy_mode {
                        if let Some(pty) = app.pty_for(session.id, pane.id) {
                            let offset = app.copy_mode.as_ref().unwrap().scroll_offset;
                            use ratatui::widgets::{Block, Borders};
                            let block = Block::default()
                                .borders(Borders::ALL)
                                .border_style(ratatui::style::Style::default().fg(ratatui::style::Color::Yellow))
                                .title(" COPY MODE -- q/esc exit  k/up up  j/dn down  g top  G bottom ");
                            let inner = block.inner(col_chunks[col]);
                            f.render_widget(block, col_chunks[col]);
                            crate::widgets::pane_view::render_scrollback(f, inner, pty, offset);
                        }
                    } else {
                        let pty = app.pty_for(session.id, pane.id);
                        if app.is_daemon_mode() {
                            render_pane_daemon(f, col_chunks[col], pane, pty, is_active);
                        } else {
                            render_pane(f, col_chunks[col], pane, pty, is_active);
                        }
                    }
                }
                pane_idx += 1;
            }
        }
    }

    if let Some(ref input) = app.rename_input {
        match app.rename_target {
            RenameTarget::Session => {
                render_rename_prompt(f, sidebar_area, "Rename session", input);
            }
            RenameTarget::Pane => {
                // find the active pane's rendered rect
                let session = app.state.sessions.get(app.state.active_session);
                if let Some(session) = session {
                    if let Some(pane) = session.panes.get(session.active_pane) {
                        if let Some(&rect) = app.pane_rects.borrow().get(&(session.id, pane.id)) {
                            render_rename_prompt(f, rect, "Rename pane", input);
                        }
                    }
                }
            }
        }
    }

    if app.show_dashboard {
        render_dashboard(f, app);
    }

    if app.show_picker {
        if let Some(picker) = &app.picker {
            crate::widgets::picker::render_picker(f, picker);
        }
    }
}

pub fn render_narrow(f: &mut Frame, app: &App) {
    use ratatui::style::{Color, Style, Stylize};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::Paragraph;

    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    let top_area = chunks[0];
    let pane_area = chunks[1];
    let bottom_area = chunks[2];

    let session = match app.state.sessions.get(app.state.active_session) {
        Some(s) => s,
        None => return,
    };

    // TOP BAR
    if app.prefix_mode {
        f.render_widget(
            Paragraph::new(" ^A› n/p:pane  N/P:sess  \":split  x:close  d:detach  D:dash")
                .style(Style::default().bg(Color::Magenta).fg(Color::White).bold()),
            top_area,
        );
    } else {
        let project = session
            .project
            .as_ref()
            .map(|p| p.name.as_str())
            .unwrap_or("—");
        let active_pane_state = session.panes.get(session.active_pane);
        let pane_name = active_pane_state.map(|p| p.name.as_str()).unwrap_or("—");
        let pane_num = session.active_pane + 1;
        let pane_total = session.panes.len();

        // Pane dots: ● for active, ○ for others (max 8 shown)
        let dots: String = (0..pane_total.min(8))
            .map(|i| {
                if i == session.active_pane {
                    "●"
                } else {
                    "○"
                }
            })
            .collect::<Vec<_>>()
            .join("");

        // Agent state colored indicator
        let (state_str, state_color) = match active_pane_state.map(|p| &p.agent_state) {
            Some(jmux_core::AgentState::Working { .. }) => ("⣾ working", Color::Blue),
            Some(jmux_core::AgentState::Waiting { .. }) => ("⚡ waiting", Color::Yellow),
            Some(jmux_core::AgentState::Error { .. }) => ("✗ error", Color::Red),
            _ => ("", Color::DarkGray),
        };

        let left = format!(
            " {} · {} · {}/{} {}",
            project, pane_name, pane_num, pane_total, dots
        );
        let right = format!(" {} ", state_str);

        let width = top_area.width as usize;
        let right_len = right.chars().count();
        let left_padded = format!("{:<width$}", left, width = width.saturating_sub(right_len));

        let line = Line::from(vec![
            Span::styled(
                left_padded,
                Style::default().bg(Color::DarkGray).fg(Color::Cyan),
            ),
            Span::styled(
                right,
                Style::default().bg(Color::DarkGray).fg(state_color).bold(),
            ),
        ]);
        f.render_widget(Paragraph::new(line), top_area);
    }

    // PANE CONTENT — no border, raw PTY output fills the area
    if let Some(pane) = session.panes.get(session.active_pane) {
        app.pane_rects.borrow_mut().clear();
        app.pane_rects
            .borrow_mut()
            .insert((session.id, pane.id), pane_area);
        let is_copy_mode = app
            .copy_mode
            .as_ref()
            .map(|cm| cm.session_id == session.id && cm.pane_id == pane.id)
            .unwrap_or(false);
        if is_copy_mode {
            if let Some(pty) = app.pty_for(session.id, pane.id) {
                let offset = app.copy_mode.as_ref().unwrap().scroll_offset;
                crate::widgets::pane_view::render_scrollback(f, pane_area, pty, offset);
            }
        } else {
            let pty = app.pty_for(session.id, pane.id);
            render_pane_content(f, pane_area, pane, pty);
        }
    }

    if let Some(ref input) = app.rename_input {
        let label = match app.rename_target {
            RenameTarget::Session => "Rename session",
            RenameTarget::Pane => "Rename pane",
        };
        render_rename_prompt(f, pane_area, label, input);
    }

    // BOTTOM BAR
    f.render_widget(
        Paragraph::new(" ^A: prefix  (n/p pane · N/P sess · \" new · x close · d detach)")
            .style(Style::default().bg(Color::Black).fg(Color::DarkGray)),
        bottom_area,
    );

    if app.show_dashboard {
        render_dashboard(f, app);
    }

    if app.show_picker {
        if let Some(picker) = &app.picker {
            crate::widgets::picker::render_picker(f, picker);
        }
    }
}

pub fn render_pane_area_for_narrow(f: &mut Frame, area: Rect, app: &App) {
    let session = match app.state.sessions.get(app.state.active_session) {
        Some(s) => s,
        None => return,
    };
    if let Some(pane) = session.panes.get(session.active_pane) {
        let pty = app.pty_for(session.id, pane.id);
        render_pane(f, area, pane, pty, true);
    }
}
