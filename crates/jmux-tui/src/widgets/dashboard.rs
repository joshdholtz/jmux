use jmux_core::AgentState;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table},
    Frame,
};

use crate::app::App;

pub fn render_dashboard(f: &mut Frame, app: &App) {
    let area = f.area();
    let rect = centered_rect(80, 80, area);

    // Enforce minimum size
    let rect = Rect {
        x: rect.x,
        y: rect.y,
        width: rect.width.max(60),
        height: rect.height.max(20),
    };

    f.render_widget(Clear, rect);

    let block = Block::default()
        .title(" ⚡ Agent Dashboard ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(rect);
    f.render_widget(block, rect);

    // Split inner area: table rows + footer line
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);

    let table_area = chunks[0];
    let footer_area = chunks[1];

    // Build table rows
    let header = Row::new(vec![
        Cell::from("SESSION").style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ),
        Cell::from("PANE").style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ),
        Cell::from("STATE").style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ),
        Cell::from("MESSAGE").style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ),
    ]);

    let mut rows: Vec<Row> = Vec::new();
    for session in &app.state.sessions {
        let session_name = session
            .project
            .as_ref()
            .map(|p| p.name.as_str())
            .unwrap_or("—");

        for pane in &session.panes {
            let (state_str, row_color) = match &pane.agent_state {
                AgentState::Waiting { .. } => ("Waiting", Color::Yellow),
                AgentState::Working { .. } => ("Working", Color::Blue),
                AgentState::Error { .. } => ("Error", Color::Red),
                AgentState::Idle => ("Idle", Color::DarkGray),
            };

            let message = match &pane.agent_state {
                AgentState::Waiting { message: Some(m) } => m.as_str(),
                AgentState::Working { message: Some(m) } => m.as_str(),
                AgentState::Error { message: Some(m) } => m.as_str(),
                _ => "",
            };

            let row_style = Style::default().fg(row_color);
            let row = Row::new(vec![
                Cell::from(session_name).style(row_style),
                Cell::from(pane.name.as_str()).style(row_style),
                Cell::from(state_str).style(row_style),
                Cell::from(message).style(row_style),
            ]);
            rows.push(row);
        }
    }

    let widths = [
        Constraint::Percentage(25),
        Constraint::Percentage(20),
        Constraint::Percentage(15),
        Constraint::Percentage(40),
    ];

    let table = Table::new(rows, widths).header(header).column_spacing(1);

    f.render_widget(table, table_area);

    // Footer
    f.render_widget(
        Paragraph::new(Line::from("esc / ctrl-a d: close  •  enter: jump to pane"))
            .style(Style::default().fg(Color::DarkGray)),
        footer_area,
    );
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
