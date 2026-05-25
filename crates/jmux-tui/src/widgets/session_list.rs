use jmux_core::AgentState;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
    Frame,
};

use crate::app::App;

pub fn render_session_list(f: &mut Frame, area: Rect, app: &App) {
    app.session_rects.borrow_mut().clear();
    app.sidebar_pane_rects.borrow_mut().clear();

    let mut items: Vec<ListItem> = Vec::new();
    let state = &app.state;

    // y tracks the row inside the bordered list (y=0 is first content row)
    let mut content_row: u16 = 0;

    for (i, session) in state.sessions.iter().enumerate() {
        let is_active_session = i == state.active_session;
        let prefix = if is_active_session { "▶ " } else { "  " };

        let project_name = session
            .project
            .as_ref()
            .map(|p| p.name.as_str())
            .unwrap_or("unnamed");

        let session_style = if is_active_session {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };

        // Record session header click rect (1 row)
        let header_rect = Rect {
            x: area.x + 1,
            y: area.y + 1 + content_row, // +1 for border
            width: area.width.saturating_sub(2),
            height: 1,
        };
        app.session_rects.borrow_mut().push(header_rect);
        content_row += 1;

        items.push(ListItem::new(Line::from(vec![Span::styled(
            format!("{}{}", prefix, project_name),
            session_style,
        )])));

        // Pane rows
        for (j, pane) in session.panes.iter().enumerate() {
            let is_active_pane = is_active_session && j == session.active_pane;

            let (state_icon, state_color) = match &pane.agent_state {
                AgentState::Working { .. } => ("⣾", Color::Blue),
                AgentState::Waiting { .. } => ("⚡", Color::Yellow),
                AgentState::Error { .. } => ("✗", Color::Red),
                AgentState::Idle => ("·", Color::DarkGray),
            };

            let pane_style = if is_active_pane {
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            // Record pane click rect (1 row)
            let pane_rect = Rect {
                x: area.x + 1,
                y: area.y + 1 + content_row,
                width: area.width.saturating_sub(2),
                height: 1,
            };
            app.sidebar_pane_rects
                .borrow_mut()
                .insert((session.id, pane.id), pane_rect);
            content_row += 1;

            items.push(ListItem::new(Line::from(vec![
                Span::styled(
                    if is_active_pane { "  ▸ " } else { "    " },
                    pane_style,
                ),
                Span::styled(
                    format!("{} ", pane.process_name.as_deref().unwrap_or(&pane.name)),
                    pane_style,
                ),
                Span::styled(state_icon, Style::default().fg(state_color)),
            ])));
        }
    }

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Sessions")
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    f.render_widget(list, area);
}
