#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_lossless, clippy::too_many_lines)]

use color_eyre::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

use crate::action::Action;
use crate::cache::store::QueryHistoryEntry;
use crate::theme;

pub struct QueryHistory {
    pub visible: bool,
    entries: Vec<QueryHistoryEntry>,
    filtered: Vec<usize>, // indices into entries
    filter_text: String,
    selected: usize,
    scroll_offset: usize,
}

impl QueryHistory {
    pub fn new() -> Self {
        Self {
            visible: false,
            entries: Vec::new(),
            filtered: Vec::new(),
            filter_text: String::new(),
            selected: 0,
            scroll_offset: 0,
        }
    }

    pub fn show(&mut self, entries: Vec<QueryHistoryEntry>) {
        self.entries = entries;
        self.filter_text.clear();
        self.selected = 0;
        self.scroll_offset = 0;
        self.visible = true;
        self.apply_filter();
    }

    pub fn hide(&mut self) {
        self.visible = false;
    }

    fn apply_filter(&mut self) {
        let query = self.filter_text.to_lowercase();
        self.filtered = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                if query.is_empty() {
                    true
                } else {
                    entry.sql.to_lowercase().contains(&query)
                }
            })
            .map(|(i, _)| i)
            .collect();
        // Keep selected in bounds
        if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len().saturating_sub(1);
        }
    }

    #[allow(clippy::unnecessary_wraps)]
    pub fn handle_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        if !self.visible {
            return Ok(None);
        }

        match key.code {
            KeyCode::Esc => {
                self.hide();
                Ok(Some(Action::Render))
            }
            KeyCode::Enter => {
                if let Some(&idx) = self.filtered.get(self.selected) {
                    let sql = self.entries[idx].sql.clone();
                    self.hide();
                    Ok(Some(Action::LoadQueryFromHistory(sql)))
                } else {
                    Ok(Some(Action::Render))
                }
            }
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                self.adjust_scroll();
                Ok(Some(Action::Render))
            }
            KeyCode::Down => {
                if self.selected + 1 < self.filtered.len() {
                    self.selected += 1;
                }
                self.adjust_scroll();
                Ok(Some(Action::Render))
            }
            KeyCode::Backspace => {
                self.filter_text.pop();
                self.apply_filter();
                Ok(Some(Action::Render))
            }
            KeyCode::Char(c) => {
                self.filter_text.push(c);
                self.apply_filter();
                Ok(Some(Action::Render))
            }
            _ => Ok(Some(Action::None)),
        }
    }

    fn adjust_scroll(&mut self) {
        // Ensure selected is visible within a ~20-item window
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + 18 {
            self.scroll_offset = self.selected.saturating_sub(17);
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        if !self.visible {
            return;
        }

        // Centered overlay
        let dialog_width = (area.width as f32 * 0.8).min(100.0) as u16;
        let dialog_height = (area.height as f32 * 0.7).min(30.0) as u16;

        let vertical = Layout::vertical([Constraint::Length(dialog_height)])
            .flex(Flex::Center)
            .split(area);
        let horizontal = Layout::horizontal([Constraint::Length(dialog_width)])
            .flex(Flex::Center)
            .split(vertical[0]);
        let dialog_area = horizontal[0];

        frame.render_widget(Clear, dialog_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::dialog_border()))
            .title(" Query History (type to filter) ")
            .title_style(Style::default().fg(theme::accent_dim()))
            .style(Style::default().bg(theme::dialog_bg()).fg(theme::fg()));

        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);

        // Split inner into: search bar (1) + list + hint bar (1)
        let chunks = Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(inner);

        // Search bar
        let search_line = Line::from(vec![
            Span::styled("> ", Style::default().fg(theme::accent())),
            Span::styled(&self.filter_text, Style::default().fg(theme::fg_bright())),
            Span::styled("_", Style::default().fg(theme::accent())),
        ]);
        let search =
            Paragraph::new(search_line).style(Style::default().bg(theme::dialog_bg()).fg(theme::fg()));
        frame.render_widget(search, chunks[0]);

        // List of matching entries
        let visible_count = chunks[1].height as usize;
        let items: Vec<ListItem> = self
            .filtered
            .iter()
            .skip(self.scroll_offset)
            .take(visible_count)
            .enumerate()
            .map(|(i, &idx)| {
                let entry = &self.entries[idx];
                let is_selected = (i + self.scroll_offset) == self.selected;
                let raw_preview: String = entry
                    .sql
                    .chars()
                    .take(60)
                    .map(|c| if c == '\n' { ' ' } else { c })
                    .collect();
                let sql_preview = if entry.sql.chars().count() > 60 {
                    format!("{raw_preview}...")
                } else {
                    raw_preview
                };

                let time_str = entry.executed_at.format("%m/%d %H:%M").to_string();
                let line = Line::from(vec![
                    Span::styled(format!("{time_str} "), Style::default().fg(theme::fg_dim())),
                    Span::styled(
                        sql_preview,
                        if is_selected {
                            Style::default()
                                .fg(theme::fg_bright())
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(theme::fg())
                        },
                    ),
                ]);

                let style = if is_selected {
                    Style::default().bg(theme::bg_highlight())
                } else {
                    Style::default().bg(theme::dialog_bg())
                };

                ListItem::new(line).style(style)
            })
            .collect();

        if items.is_empty() {
            let empty_msg = if self.filter_text.is_empty() {
                "No query history"
            } else {
                "No matching queries"
            };
            let empty = Paragraph::new(Span::styled(empty_msg, Style::default().fg(theme::fg_dim())))
                .alignment(ratatui::layout::Alignment::Center)
                .style(Style::default().bg(theme::dialog_bg()).fg(theme::fg()));
            frame.render_widget(empty, chunks[1]);
        } else {
            let list = List::new(items);
            frame.render_widget(list, chunks[1]);
        }

        // Hint bar
        let accent = Style::default().fg(theme::accent_dim());
        let dim = Style::default().fg(theme::fg_dim());
        let hint_line = Line::from(vec![
            Span::styled(" Enter", accent),
            Span::styled(": Load  ", dim),
            Span::styled("Esc", accent),
            Span::styled(": Close  ", dim),
            Span::styled("Type to filter", dim),
        ]);
        let hint_bar = Paragraph::new(hint_line).style(Style::default().bg(theme::dialog_bg()));
        frame.render_widget(hint_bar, chunks[2]);
    }
}
