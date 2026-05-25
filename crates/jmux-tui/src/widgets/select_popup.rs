use jmux_core::PendingSelect;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
    Frame,
};

pub fn render_select_popup(f: &mut Frame, sel: &PendingSelect) {
    let area = centered_rect(60, 50, f.area());
    f.render_widget(ratatui::widgets::Clear, area);

    let block = Block::default()
        .title(" Select ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let visible = inner.height as usize;
    let count = sel.items.len();

    // Compute scroll offset so the selected item is always visible
    let scroll_offset = if sel.selected >= visible {
        sel.selected + 1 - visible
    } else {
        0
    };

    let show_hint = count > visible;

    let list_area = if show_hint {
        // Reserve 1 line at the bottom for the hint
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(inner);
        // Draw the hint
        let hint_text = format!(
            " ({}/{}) ↑↓/j/k navigate · enter select · esc cancel",
            sel.selected + 1,
            count
        );
        f.render_widget(
            ratatui::widgets::Paragraph::new(hint_text).style(Style::default().fg(Color::DarkGray)),
            chunks[1],
        );
        chunks[0]
    } else {
        inner
    };

    let end = (scroll_offset + list_area.height as usize).min(count);
    let list_items: Vec<ListItem> = (scroll_offset..end)
        .map(|i| {
            let is_selected = i == sel.selected;
            let style = if is_selected {
                Style::default()
                    .bg(Color::Cyan)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let prefix = if is_selected { " ▶ " } else { "   " };
            let label = format!("{}{}", prefix, sel.items[i]);
            ListItem::new(Line::from(Span::styled(label, style)))
        })
        .collect();

    f.render_widget(List::new(list_items), list_area);
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
