use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::Paragraph,
    Frame,
};

pub fn render_rename_prompt(f: &mut Frame, pane_area: Rect, label: &str, input: &str) {
    if pane_area.height < 3 {
        return;
    }
    // Render a 1-line prompt at the bottom of the pane area (overlaid)
    let prompt_area = Rect {
        x: pane_area.x,
        y: pane_area.y + pane_area.height - 1,
        width: pane_area.width,
        height: 1,
    };
    let text = format!(" {}: {}_ ", label, input);
    f.render_widget(
        Paragraph::new(text).style(Style::default().bg(Color::Blue).fg(Color::White)),
        prompt_area,
    );
}
