use std::path::PathBuf;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

pub struct PickerState {
    pub query: String,
    pub items: Vec<PickerItem>,
    pub filtered: Vec<usize>, // indices into items
    pub selected: usize,
}

pub struct PickerItem {
    pub label: String, // display name
    pub path: PathBuf,
    pub is_open: bool, // already an open session
}

impl PickerState {
    pub fn new(items: Vec<PickerItem>) -> Self {
        let filtered: Vec<usize> = (0..items.len()).collect();
        Self {
            query: String::new(),
            items,
            filtered,
            selected: 0,
        }
    }

    pub fn update_query(&mut self, query: String) {
        self.query = query.clone();
        let q = query.to_lowercase();
        self.filtered = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| q.is_empty() || item.label.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
        self.selected = 0;
    }

    pub fn move_up(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = (self.selected + self.filtered.len() - 1) % self.filtered.len();
        }
    }

    pub fn move_down(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = (self.selected + 1) % self.filtered.len();
        }
    }

    pub fn selected_item(&self) -> Option<&PickerItem> {
        self.filtered
            .get(self.selected)
            .and_then(|&i| self.items.get(i))
    }
}

pub fn scan_projects(open_sessions: &[jmux_core::Session]) -> Vec<PickerItem> {
    let home = dirs_next::home_dir().unwrap_or_else(|| PathBuf::from("/"));

    let search_dirs = ["Developer", "dev", "projects", "code", "src"];

    let mut items: Vec<PickerItem> = Vec::new();

    // Collect open session roots for de-duplication and marking
    let open_paths: Vec<PathBuf> = open_sessions
        .iter()
        .filter_map(|s| s.project.as_ref().map(|p| p.root.clone()))
        .collect();

    // Add open sessions at the top first
    for session in open_sessions {
        if let Some(project) = &session.project {
            let already_added = items.iter().any(|i| i.path == project.root);
            if !already_added {
                items.push(PickerItem {
                    label: project.name.clone(),
                    path: project.root.clone(),
                    is_open: true,
                });
            }
        }
    }

    let project_markers = [
        ".git",
        "Cargo.toml",
        "package.json",
        "go.mod",
        "pyproject.toml",
    ];

    for dir_name in &search_dirs {
        let search_root = home.join(dir_name);
        if !search_root.exists() {
            continue;
        }

        let entries = match std::fs::read_dir(&search_root) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            if items.len() >= 200 {
                break;
            }

            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            // Check if it contains any project marker
            let is_project = project_markers
                .iter()
                .any(|marker| path.join(marker).exists());

            if !is_project {
                continue;
            }

            // Skip if already added (open session)
            if items.iter().any(|i| i.path == path) {
                continue;
            }

            let label = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            let is_open = open_paths.contains(&path);

            items.push(PickerItem {
                label,
                path,
                is_open,
            });
        }
    }

    items
}

pub fn render_picker(f: &mut Frame, state: &PickerState) {
    let area = centered_rect(70, 60, f.area());
    f.render_widget(ratatui::widgets::Clear, area);

    let block = Block::default()
        .title(" Open Project ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Layout: search box (3 lines with border) + list
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(inner);

    // Search box
    let search_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(" search ");
    let search_text = format!("{}_", state.query); // underscore as cursor
    f.render_widget(
        Paragraph::new(search_text)
            .block(search_block)
            .style(Style::default().fg(Color::White)),
        chunks[0],
    );

    // Items list
    let list_items: Vec<ListItem> = state
        .filtered
        .iter()
        .enumerate()
        .map(|(display_i, &item_i)| {
            let item = &state.items[item_i];
            let is_selected = display_i == state.selected;
            let prefix = if item.is_open { "▶ " } else { "  " };
            let label = format!("{}{}", prefix, item.label);
            let style = if is_selected {
                Style::default()
                    .bg(Color::Cyan)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD)
            } else if item.is_open {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(Span::styled(label, style)))
        })
        .collect();

    f.render_widget(List::new(list_items), chunks[1]);
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
