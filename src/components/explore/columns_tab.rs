#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_lossless, clippy::too_many_lines)]

use std::cell::Cell;

use color_eyre::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell as TableCell, Paragraph, Row, Table as TableWidget};

use crate::action::Action;
use crate::components::Component;
use crate::snowflake::Column;
use crate::theme;

/// Left/right padding width
const LEFT_PAD: u16 = 1;
const RIGHT_PAD: u16 = 1;

pub struct ColumnsTab {
    columns: Vec<Column>,
    /// Indices into `columns` that match the current search filter
    filtered_indices: Vec<usize>,
    selected: usize,
    offset: usize,
    visible_height: Cell<usize>,
    loading: bool,
    error: Option<String>,
    /// Column name search
    search_query: String,
    search_active_flag: bool,
}

impl ColumnsTab {
    pub fn new() -> Self {
        Self {
            columns: Vec::new(),
            filtered_indices: Vec::new(),
            selected: 0,
            offset: 0,
            visible_height: Cell::new(20),
            loading: false,
            error: None,
            search_query: String::new(),
            search_active_flag: false,
        }
    }

    pub fn set_loading(&mut self) {
        self.loading = true;
        self.error = None;
    }

    pub fn set_columns(&mut self, columns: Vec<Column>) {
        self.columns = columns;
        self.selected = 0;
        self.offset = 0;
        self.loading = false;
        self.error = None;
        self.search_query.clear();
        self.search_active_flag = false;
        self.rebuild_filter();
    }

    pub fn set_error(&mut self, msg: String) {
        self.error = Some(msg);
        self.loading = false;
    }

    pub fn activate_search(&mut self) {
        self.search_active_flag = true;
        self.search_query.clear();
        self.rebuild_filter();
    }

    pub fn search_active(&self) -> bool {
        self.search_active_flag
    }

    fn deactivate_search(&mut self) {
        self.search_active_flag = false;
        self.search_query.clear();
        self.rebuild_filter();
        self.selected = 0;
        self.offset = 0;
    }

    fn rebuild_filter(&mut self) {
        if self.search_query.is_empty() {
            self.filtered_indices = (0..self.columns.len()).collect();
        } else {
            let query_lower = self.search_query.to_lowercase();
            self.filtered_indices = self
                .columns
                .iter()
                .enumerate()
                .filter(|(_, col)| col.name.to_lowercase().contains(&query_lower))
                .map(|(i, _)| i)
                .collect();
        }
        // Reset selection when filter changes
        self.selected = 0;
        self.offset = 0;
    }

    /// Ensure selected row is visible within the scroll window
    fn adjust_offset(&mut self) {
        let visible = self.visible_height.get();
        if visible == 0 {
            return;
        }
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + visible {
            self.offset = self.selected - visible + 1;
        }
    }
}

impl Component for ColumnsTab {
    fn update(&mut self, _action: Action) -> Result<Option<Action>> {
        Ok(None)
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        // Handle search input when active
        if self.search_active_flag {
            match key.code {
                KeyCode::Esc => {
                    self.deactivate_search();
                    return Ok(Some(Action::Render));
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                    self.rebuild_filter();
                    return Ok(Some(Action::Render));
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                    self.rebuild_filter();
                    return Ok(Some(Action::Render));
                }
                // Navigation keys (Down/Up/j/k) fall through to the navigation section below
                KeyCode::Down | KeyCode::Up => {}
                _ => {
                    // All other keys are consumed by search (don't trigger tab switching etc.)
                    return Ok(None);
                }
            }
        }

        if self.filtered_indices.is_empty() {
            return Ok(None);
        }
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected < self.filtered_indices.len().saturating_sub(1) {
                    self.selected += 1;
                    self.adjust_offset();
                }
                Ok(Some(Action::Render))
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                self.adjust_offset();
                Ok(Some(Action::Render))
            }
            _ => Ok(None),
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Add left + right padding
        let h_pad = LEFT_PAD + RIGHT_PAD;
        if area.width <= h_pad || area.height < 1 {
            return;
        }
        let padded_area = Rect {
            x: area.x + LEFT_PAD,
            y: area.y,
            width: area.width - h_pad,
            height: area.height,
        };

        let block = Block::default()
            .borders(Borders::NONE)
            .style(Style::default().bg(theme::bg_surface()).fg(theme::fg()));

        if self.loading {
            let paragraph = Paragraph::new("Loading columns...")
                .style(Style::default().fg(theme::fg_dim()))
                .block(block);
            frame.render_widget(paragraph, padded_area);
            return;
        }

        if let Some(ref err) = self.error {
            let paragraph = Paragraph::new(format!("Error: {err}"))
                .style(Style::default().fg(theme::red()))
                .block(block);
            frame.render_widget(paragraph, padded_area);
            return;
        }

        if self.columns.is_empty() {
            let paragraph = Paragraph::new("Select a table to view columns")
                .style(Style::default().fg(theme::fg_dim()))
                .block(block);
            frame.render_widget(paragraph, padded_area);
            return;
        }

        // Determine how many rows to reserve: header(1) + search bar(1 if active)
        let search_height: u16 = u16::from(self.search_active_flag);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),             // header
                Constraint::Min(0),                // body
                Constraint::Length(search_height), // search bar
            ])
            .split(padded_area);

        // Render header (all caps) with per-cell bg so gap reveals dark BG
        let header_cells: Vec<TableCell> = ["NAME", "TYPE", "NULLABLE"]
            .iter()
            .map(|&label| {
                TableCell::from(label).style(
                    Style::default()
                        .fg(theme::accent_dim())
                        .bg(theme::bg_surface())
                        .add_modifier(Modifier::BOLD),
                )
            })
            .collect();
        let header = Row::new(header_cells);

        let header_widths = [
            Constraint::Percentage(35),
            Constraint::Percentage(45),
            Constraint::Percentage(20),
        ];
        let header_table = TableWidget::new(vec![header], header_widths)
            .column_spacing(1)
            .block(Block::default().style(Style::default().bg(theme::bg())));
        frame.render_widget(header_table, chunks[0]);

        // Update visible height for scroll offset calculations
        self.visible_height.set(chunks[1].height as usize);

        // Build rows from filtered indices
        let rows: Vec<Row> = self
            .filtered_indices
            .iter()
            .enumerate()
            .skip(self.offset)
            .map(|(fi, &col_idx)| {
                let col = &self.columns[col_idx];
                let nullable_str = if col.nullable { "YES" } else { "NO" };
                let is_selected = fi == self.selected;
                let bg = if is_selected {
                    theme::bg_highlight()
                } else {
                    theme::bg_surface()
                };
                let fg = if is_selected {
                    theme::fg_bright()
                } else {
                    theme::fg()
                };
                let values = [col.name.as_str(), col.data_type.as_str(), nullable_str];
                let cells: Vec<TableCell> = values
                    .iter()
                    .map(|&val| {
                        TableCell::from(val.to_string()).style(Style::default().fg(fg).bg(bg))
                    })
                    .collect();
                Row::new(cells)
            })
            .collect();

        if self.filtered_indices.is_empty() && !self.search_query.is_empty() {
            let no_match = Paragraph::new("No matching columns")
                .style(Style::default().fg(theme::fg_dim()).bg(theme::bg_surface()));
            frame.render_widget(no_match, chunks[1]);
        } else {
            let widths = [
                Constraint::Percentage(35),
                Constraint::Percentage(45),
                Constraint::Percentage(20),
            ];

            let body_block = Block::default()
                .borders(Borders::NONE)
                .style(Style::default().bg(theme::bg()).fg(theme::fg()));

            let table = TableWidget::new(rows, widths)
                .column_spacing(1)
                .block(body_block);

            frame.render_widget(table, chunks[1]);
        }

        // Search bar at bottom
        if self.search_active_flag && chunks[2].height > 0 {
            let search_line = Line::from(vec![
                Span::styled(
                    " / ",
                    Style::default().fg(theme::accent()).bg(theme::bg_surface()),
                ),
                Span::styled(
                    self.search_query.as_str(),
                    Style::default().fg(theme::fg_bright()).bg(theme::bg_surface()),
                ),
                Span::styled(
                    "\u{2588}",
                    Style::default().fg(theme::accent()).bg(theme::bg_surface()),
                ),
            ]);
            let search_paragraph = Paragraph::new(search_line)
                .style(Style::default().bg(theme::bg_surface()).fg(theme::fg()));
            frame.render_widget(search_paragraph, chunks[2]);
        }
    }
}
