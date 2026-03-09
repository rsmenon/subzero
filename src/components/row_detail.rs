#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_lossless, clippy::too_many_lines)]

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::components::format_value;
use crate::theme;

/// A scrollable popup that displays all column-value pairs for a single row.
pub struct RowDetailPopup {
    pub visible: bool,
    /// All entries: (`column_name`, `formatted_value`, `is_null`)
    entries: Vec<(String, String, bool)>,
    /// Filtered indices into `entries` (when search is active, only these are shown)
    filtered: Vec<usize>,
    scroll_offset: usize,
    /// Search state: None = not searching, Some(query) = filtering
    search: Option<String>,
}

impl RowDetailPopup {
    pub fn new() -> Self {
        Self {
            visible: false,
            entries: Vec::new(),
            filtered: Vec::new(),
            scroll_offset: 0,
            search: None,
        }
    }

    /// Show the popup with data from the given columns and row values.
    pub fn show(&mut self, columns: &[String], row: &[serde_json::Value]) {
        self.entries = columns
            .iter()
            .enumerate()
            .map(|(i, col)| {
                let value = row.get(i);
                let is_null = matches!(value, Some(serde_json::Value::Null) | None);
                let null_val = serde_json::Value::Null;
                let formatted = format_value(value.unwrap_or(&null_val));
                (col.clone(), formatted, is_null)
            })
            .collect();
        self.filtered = (0..self.entries.len()).collect();
        self.scroll_offset = 0;
        self.search = None;
        self.visible = true;
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.search = None;
    }

    /// Recompute filtered indices based on current search query.
    fn refilter(&mut self) {
        let query = match &self.search {
            Some(q) if !q.is_empty() => q.to_lowercase(),
            _ => {
                self.filtered = (0..self.entries.len()).collect();
                return;
            }
        };
        self.filtered = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, (col, _, _))| col.to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect();
        // Clamp scroll offset
        if self.filtered.is_empty() {
            self.scroll_offset = 0;
        } else {
            self.scroll_offset = self
                .scroll_offset
                .min(self.filtered.len().saturating_sub(1));
        }
    }

    /// Handle key events. Always returns true (all keys consumed while popup is open).
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        // If search is active, handle search-specific keys
        if self.search.is_some() {
            match key.code {
                KeyCode::Esc => {
                    // Close search, show all entries
                    self.search = None;
                    self.refilter();
                    self.scroll_offset = 0;
                    return true;
                }
                KeyCode::Enter => {
                    // Confirm search and return to browse mode
                    self.search = None;
                    // Keep filtered results
                    return true;
                }
                KeyCode::Backspace => {
                    if let Some(ref mut q) = self.search {
                        q.pop();
                        if q.is_empty() {
                            self.search = None;
                        }
                    }
                    self.refilter();
                    return true;
                }
                KeyCode::Char(c) => {
                    if let Some(ref mut q) = self.search {
                        q.push(c);
                    }
                    self.refilter();
                    return true;
                }
                _ => return true,
            }
        }

        // Normal browsing mode
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                let max = self.filtered.len().saturating_sub(1);
                if self.scroll_offset < max {
                    self.scroll_offset += 1;
                }
                true
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
                true
            }
            KeyCode::PageDown => {
                let max = self.filtered.len().saturating_sub(1);
                self.scroll_offset = (self.scroll_offset + 10).min(max);
                true
            }
            KeyCode::PageUp => {
                self.scroll_offset = self.scroll_offset.saturating_sub(10);
                true
            }
            KeyCode::Home => {
                self.scroll_offset = 0;
                true
            }
            KeyCode::End => {
                self.scroll_offset = self.filtered.len().saturating_sub(1);
                true
            }
            KeyCode::Char('/') => {
                self.search = Some(String::new());
                true
            }
            KeyCode::Esc | KeyCode::Enter => {
                self.hide();
                true
            }
            _ => true,
        }
    }

    /// Render the popup centered over the given area.
    /// `full_area` should be the full screen/terminal area for proper vertical centering.
    pub fn render(&self, frame: &mut Frame, area: Rect, full_area: Rect) {
        if !self.visible || self.entries.is_empty() {
            return;
        }

        // Width: 80% of local area (max 100), Height: 70% of full screen height (max 30)
        let width = ((area.width as f32 * 0.8) as u16).clamp(30, 100);
        let height = ((full_area.height as f32 * 0.7) as u16).clamp(8, 30);

        let vert = Layout::vertical([Constraint::Length(height)])
            .flex(Flex::Center)
            .split(full_area);
        let popup_area = Layout::horizontal([Constraint::Length(width)])
            .flex(Flex::Center)
            .split(vert[0])[0];

        frame.render_widget(Clear, popup_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::dialog_border()))
            .title(" Row Detail ")
            .title_style(Style::default().fg(theme::accent_dim()))
            .style(Style::default().bg(theme::dialog_bg()).fg(theme::fg()));

        let inner = block.inner(popup_area);
        frame.render_widget(block, popup_area);

        if inner.height < 3 || inner.width < 4 {
            return;
        }

        // Split inner: content + hint bar at bottom
        let inner_chunks =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(inner);
        let content_area = inner_chunks[0];
        let hint_area = inner_chunks[1];

        // Render hint bar
        let hint_line = if let Some(ref query) = self.search {
            Line::from(vec![
                Span::styled(" /", Style::default().fg(theme::accent())),
                Span::styled(query, Style::default().fg(theme::fg_bright())),
                Span::styled("\u{2588}", Style::default().fg(theme::fg_dim())),
            ])
        } else {
            Line::from(Span::styled(
                " Esc: close | j/k: scroll | /: search ",
                Style::default().fg(theme::fg_dim()),
            ))
        };
        let hint_paragraph = Paragraph::new(hint_line).style(Style::default().bg(theme::dialog_bg()));
        frame.render_widget(hint_paragraph, hint_area);

        if self.filtered.is_empty() {
            let msg = if self.search.is_some() {
                "No matching columns"
            } else {
                "No data"
            };
            let paragraph = Paragraph::new(Line::from(Span::styled(
                msg,
                Style::default().fg(theme::fg_dim()),
            )))
            .style(Style::default().bg(theme::dialog_bg()));
            frame.render_widget(paragraph, content_area);
            return;
        }

        // Find the longest column name (among filtered) for alignment
        let max_col_width = self
            .filtered
            .iter()
            .map(|&i| self.entries[i].0.chars().count())
            .max()
            .unwrap_or(0)
            .min((content_area.width as usize).saturating_sub(4) / 2);

        // Build lines for visible entries
        let visible_count = content_area.height as usize;
        let lines: Vec<Line> = self
            .filtered
            .iter()
            .enumerate()
            .skip(self.scroll_offset)
            .take(visible_count)
            .map(|(fi, &entry_idx)| {
                let (ref col, ref val, is_null) = self.entries[entry_idx];
                let col_display: String = if col.chars().count() > max_col_width {
                    let mut s: String = col.chars().take(max_col_width.saturating_sub(1)).collect();
                    s.push('\u{2026}');
                    s
                } else {
                    col.clone()
                };
                let padding = max_col_width.saturating_sub(col_display.chars().count());
                let pad_str = " ".repeat(padding);

                // Highlight the selected/scrolled-to entry
                let is_current = fi == self.scroll_offset;

                let col_style = if is_current {
                    Style::default()
                        .fg(theme::accent())
                        .bg(theme::dialog_bg())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme::fg_dim()).bg(theme::dialog_bg())
                };

                let val_style = if is_null {
                    Style::default()
                        .fg(theme::fg_dim())
                        .bg(theme::dialog_bg())
                        .add_modifier(Modifier::ITALIC)
                } else if is_current {
                    Style::default().fg(theme::fg_bright()).bg(theme::dialog_bg())
                } else {
                    Style::default().fg(theme::fg()).bg(theme::dialog_bg())
                };

                Line::from(vec![
                    Span::styled(" ", Style::default().bg(theme::dialog_bg())),
                    Span::styled(col_display, col_style),
                    Span::styled(pad_str, Style::default().bg(theme::dialog_bg())),
                    Span::styled("  ", Style::default().bg(theme::dialog_bg())),
                    Span::styled(val, val_style),
                ])
            })
            .collect();

        let paragraph = Paragraph::new(lines)
            .style(Style::default().bg(theme::dialog_bg()).fg(theme::fg()))
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, content_area);
    }
}
