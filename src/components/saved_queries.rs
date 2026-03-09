#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_lossless, clippy::too_many_lines)]

use color_eyre::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell as TableCell, Clear, Row, Table as TableWidget};

use crate::action::Action;
use crate::cache::store::SavedQuery;
use crate::theme;

pub struct SavedQueriesPane {
    pub visible: bool,
    pub queries: Vec<SavedQuery>,
    selected: usize,
    scroll_offset: usize,
    editing_title: bool,
    edit_buffer: String,
    delete_confirm: bool,
}

impl SavedQueriesPane {
    pub fn new() -> Self {
        Self {
            visible: false,
            queries: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            editing_title: false,
            edit_buffer: String::new(),
            delete_confirm: false,
        }
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    pub fn set_queries(&mut self, queries: Vec<SavedQuery>) {
        self.queries = queries;
        if self.selected >= self.queries.len() {
            self.selected = self.queries.len().saturating_sub(1);
        }
    }

    #[allow(clippy::unnecessary_wraps)]
    pub fn handle_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        if !self.visible {
            return Ok(None);
        }

        // Delete confirmation mode
        if self.delete_confirm {
            if let KeyCode::Char('y' | 'Y') = key.code {
                self.delete_confirm = false;
                if !self.queries.is_empty() {
                    self.queries.remove(self.selected);
                    if self.selected >= self.queries.len() {
                        self.selected = self.queries.len().saturating_sub(1);
                    }
                    return Ok(Some(Action::PersistSavedQueries));
                }
                return Ok(Some(Action::Render));
            }
            // Any other key cancels
            self.delete_confirm = false;
            return Ok(Some(Action::Render));
        }

        // Editing title mode
        if self.editing_title {
            match key.code {
                KeyCode::Enter => {
                    // Save the edited title
                    if let Some(q) = self.queries.get_mut(self.selected) {
                        q.title.clone_from(&self.edit_buffer);
                    }
                    self.editing_title = false;
                    // Persist the change
                    return Ok(Some(Action::PersistSavedQueries));
                }
                KeyCode::Esc => {
                    self.editing_title = false;
                    return Ok(Some(Action::Render));
                }
                KeyCode::Backspace => {
                    self.edit_buffer.pop();
                    return Ok(Some(Action::Render));
                }
                KeyCode::Char(c) => {
                    self.edit_buffer.push(c);
                    return Ok(Some(Action::Render));
                }
                _ => return Ok(Some(Action::None)),
            }
        }

        match key.code {
            KeyCode::Esc => {
                self.visible = false;
                Ok(Some(Action::Render))
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.queries.is_empty() && self.selected + 1 < self.queries.len() {
                    self.selected += 1;
                }
                self.adjust_scroll();
                Ok(Some(Action::Render))
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                self.adjust_scroll();
                Ok(Some(Action::Render))
            }
            KeyCode::Enter => {
                if let Some(q) = self.queries.get(self.selected) {
                    let sql = q.sql.clone();
                    let id = q.id;
                    self.visible = false;
                    return Ok(Some(Action::LoadSavedQuery { id, sql }));
                }
                Ok(Some(Action::Render))
            }
            KeyCode::Char('e') => {
                // Start editing the title
                if let Some(q) = self.queries.get(self.selected) {
                    self.edit_buffer = q.title.clone();
                    self.editing_title = true;
                }
                Ok(Some(Action::Render))
            }
            KeyCode::Char('d') => {
                // Request delete confirmation
                if !self.queries.is_empty() {
                    self.delete_confirm = true;
                }
                Ok(Some(Action::Render))
            }
            _ => Ok(Some(Action::None)),
        }
    }

    fn adjust_scroll(&mut self) {
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
            .title(" Saved Queries ")
            .title_style(Style::default().fg(theme::accent_dim()))
            .style(Style::default().bg(theme::dialog_bg()).fg(theme::fg()));

        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);

        if inner.height < 3 || inner.width < 4 {
            return;
        }

        // Split inner: content + hint bar at bottom
        let inner_chunks = Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(inner);
        let content_area = inner_chunks[0];
        let hint_area = inner_chunks[1];

        // Render hint bar
        let accent = Style::default().fg(theme::accent_dim());
        let dim = Style::default().fg(theme::fg_dim());
        let hint_line = if self.delete_confirm {
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(" Delete this query? ", dim),
                ratatui::text::Span::styled("Y", accent),
                ratatui::text::Span::styled(": Confirm  ", dim),
                ratatui::text::Span::styled("any key", accent),
                ratatui::text::Span::styled(": Cancel", dim),
            ])
        } else if self.editing_title {
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(" Enter", accent),
                ratatui::text::Span::styled(": Confirm  ", dim),
                ratatui::text::Span::styled("Esc", accent),
                ratatui::text::Span::styled(": Cancel", dim),
            ])
        } else {
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(" e", accent),
                ratatui::text::Span::styled(": Edit title  ", dim),
                ratatui::text::Span::styled("d", accent),
                ratatui::text::Span::styled(": Delete  ", dim),
                ratatui::text::Span::styled("Enter", accent),
                ratatui::text::Span::styled(": Load  ", dim),
                ratatui::text::Span::styled("Esc", accent),
                ratatui::text::Span::styled(": Close", dim),
            ])
        };
        let hint_paragraph = ratatui::widgets::Paragraph::new(hint_line)
            .style(Style::default().bg(theme::dialog_bg()));
        frame.render_widget(hint_paragraph, hint_area);

        if self.queries.is_empty() {
            let paragraph = ratatui::widgets::Paragraph::new(
                "No saved queries. Use Ctrl+S in the query editor to save.",
            )
            .style(Style::default().fg(theme::fg_dim()).bg(theme::dialog_bg()));
            frame.render_widget(paragraph, content_area);
            return;
        }

        // Header with per-cell bg so gap reveals dark separator
        let header_labels = ["SAVED AT", "TITLE", "SQL PREVIEW", "LAST RUN"];
        let header_cells: Vec<TableCell> = header_labels
            .iter()
            .map(|&label| {
                TableCell::from(label).style(
                    Style::default()
                        .fg(theme::accent_dim())
                        .bg(theme::dialog_bg())
                        .add_modifier(Modifier::BOLD),
                )
            })
            .collect();
        let header = Row::new(header_cells);

        let visible_count = content_area.height.saturating_sub(2) as usize;
        let rows: Vec<Row> = self
            .queries
            .iter()
            .skip(self.scroll_offset)
            .take(visible_count)
            .enumerate()
            .map(|(i, q)| {
                let is_selected = (i + self.scroll_offset) == self.selected;
                let bg = if is_selected {
                    theme::bg_highlight()
                } else {
                    theme::dialog_bg()
                };
                let fg = if is_selected {
                    theme::fg_bright()
                } else {
                    theme::fg()
                };

                let saved_at = q.saved_at.format("%m/%d %H:%M").to_string();
                let last_run = q
                    .last_run_at.map_or_else(|| "-".to_string(), |t| t.format("%m/%d %H:%M").to_string());

                let title_text = if is_selected && self.editing_title {
                    format!("{}|", self.edit_buffer)
                } else {
                    q.title.clone()
                };

                let raw_preview: String = q
                    .sql
                    .chars()
                    .take(40)
                    .map(|c| if c == '\n' { ' ' } else { c })
                    .collect();
                let sql_preview = if q.sql.chars().count() > 40 {
                    format!("{raw_preview}...")
                } else {
                    raw_preview
                };

                let values = [saved_at, title_text, sql_preview, last_run];
                let cells: Vec<TableCell> = values
                    .iter()
                    .map(|val| TableCell::from(val.clone()).style(Style::default().fg(fg).bg(bg)))
                    .collect();

                Row::new(cells)
            })
            .collect();

        let widths = [
            Constraint::Length(14),
            Constraint::Length(20),
            Constraint::Min(20),
            Constraint::Length(14),
        ];

        let table = TableWidget::new(rows, widths)
            .header(header)
            .column_spacing(1)
            .style(Style::default().bg(theme::bg_surface()).fg(theme::fg()));

        frame.render_widget(table, content_area);
    }
}
