#![allow(clippy::cast_possible_truncation, clippy::too_many_lines)]

use color_eyre::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table};

use crate::action::Action;
use crate::components::format_value;
use crate::components::row_detail::RowDetailPopup;
use crate::snowflake::QueryResult;
use crate::theme;

/// Left/right padding width
const LEFT_PAD: u16 = 1;
const RIGHT_PAD: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultsState {
    Empty,
    Loading,
    Loaded,
    Error(String),
}

pub struct ResultsTable {
    pub state: ResultsState,
    pub result: Option<QueryResult>,
    pub selected_row: usize,
    pub row_offset: usize,
    pub scroll_col: usize,
    pub focused: bool,
    visible_height: usize,
    pub row_detail: RowDetailPopup,
    /// Pre-formatted cell values (computed once on `set_result`, not per frame)
    formatted_cells: Vec<Vec<String>>,
}

impl ResultsTable {
    pub fn new() -> Self {
        Self {
            state: ResultsState::Empty,
            result: None,
            selected_row: 0,
            row_offset: 0,
            scroll_col: 0,
            focused: false,
            visible_height: 20,
            row_detail: RowDetailPopup::new(),
            formatted_cells: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.state = ResultsState::Empty;
        self.result = None;
        self.formatted_cells.clear();
        self.selected_row = 0;
        self.row_offset = 0;
        self.scroll_col = 0;
    }

    pub fn set_loading(&mut self) {
        self.state = ResultsState::Loading;
        self.result = None;
        self.formatted_cells.clear();
        self.selected_row = 0;
        self.row_offset = 0;
        self.scroll_col = 0;
    }

    pub fn set_result(&mut self, result: QueryResult) {
        self.state = ResultsState::Loaded;
        // Pre-format all cells once
        self.formatted_cells = result
            .rows
            .iter()
            .map(|row| row.iter().map(format_value).collect())
            .collect();
        self.result = Some(result);
        self.selected_row = 0;
        self.row_offset = 0;
        self.scroll_col = 0;
    }

    pub fn set_error(&mut self, error: String) {
        self.state = ResultsState::Error(error);
        self.result = None;
        self.formatted_cells.clear();
    }

    fn max_selected_row(&self) -> usize {
        if let Some(ref result) = self.result {
            result.rows.len().saturating_sub(1)
        } else {
            0
        }
    }

    fn max_scroll_col(&self) -> usize {
        if let Some(ref result) = self.result {
            result.columns.len().saturating_sub(1)
        } else {
            0
        }
    }

    /// Ensure selected row is visible within the scroll window
    fn adjust_row_offset(&mut self) {
        // Subtract 1 for header row
        let visible = self.visible_height.saturating_sub(1);
        if visible == 0 {
            return;
        }
        if self.selected_row < self.row_offset {
            self.row_offset = self.selected_row;
        } else if self.selected_row >= self.row_offset + visible {
            self.row_offset = self.selected_row - visible + 1;
        }
    }

    #[allow(clippy::unnecessary_wraps)]
    pub fn handle_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        // Row detail popup captures all input when visible
        if self.row_detail.visible {
            self.row_detail.handle_key(key);
            return Ok(Some(Action::Render));
        }

        match key.code {
            KeyCode::Enter => {
                if let Some(ref result) = self.result
                    && let Some(row) = result.rows.get(self.selected_row)
                {
                    self.row_detail.show(&result.columns, row);
                }
                Ok(Some(Action::Render))
            }
            // Row selection (move selected row)
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected_row < self.max_selected_row() {
                    self.selected_row += 1;
                    self.adjust_row_offset();
                }
                Ok(Some(Action::Render))
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected_row = self.selected_row.saturating_sub(1);
                self.adjust_row_offset();
                Ok(Some(Action::Render))
            }
            KeyCode::PageDown => {
                self.selected_row = (self.selected_row + 20).min(self.max_selected_row());
                self.adjust_row_offset();
                Ok(Some(Action::Render))
            }
            KeyCode::PageUp => {
                self.selected_row = self.selected_row.saturating_sub(20);
                self.adjust_row_offset();
                Ok(Some(Action::Render))
            }
            KeyCode::Home => {
                self.selected_row = 0;
                self.adjust_row_offset();
                Ok(Some(Action::Render))
            }
            KeyCode::End => {
                self.selected_row = self.max_selected_row();
                self.adjust_row_offset();
                Ok(Some(Action::Render))
            }
            // Horizontal scrolling
            KeyCode::Right | KeyCode::Char('l') => {
                if self.scroll_col < self.max_scroll_col() {
                    self.scroll_col += 1;
                }
                Ok(Some(Action::Render))
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.scroll_col = self.scroll_col.saturating_sub(1);
                Ok(Some(Action::Render))
            }
            _ => Ok(None),
        }
    }

    pub fn render_content(&mut self, frame: &mut Frame, area: Rect) {
        if area.height < 1 || area.width < 4 {
            return;
        }

        match &self.state {
            ResultsState::Empty => {
                let text = Paragraph::new(Line::from(Span::styled(
                    "Execute a query to see results",
                    Style::default().fg(theme::fg_dim()),
                )))
                .style(Style::default().bg(theme::bg_surface()))
                .alignment(ratatui::layout::Alignment::Center);
                frame.render_widget(text, area);
            }
            ResultsState::Loading => {
                let text = Paragraph::new(Line::from(Span::styled(
                    "Loading...",
                    Style::default()
                        .fg(theme::yellow())
                        .add_modifier(Modifier::BOLD),
                )))
                .style(Style::default().bg(theme::bg_surface()))
                .alignment(ratatui::layout::Alignment::Center);
                frame.render_widget(text, area);
            }
            ResultsState::Error(err) => {
                let text = Paragraph::new(Line::from(Span::styled(
                    err.as_str(),
                    Style::default().fg(theme::red()),
                )))
                .style(Style::default().bg(theme::bg_surface()))
                .wrap(ratatui::widgets::Wrap { trim: false });
                frame.render_widget(text, area);
            }
            ResultsState::Loaded => {
                self.render_table(frame, area);
            }
        }
    }

    fn render_table(&mut self, frame: &mut Frame, area: Rect) {
        let Some(result) = &self.result else {
            return;
        };

        // Add left + right padding
        let h_pad = LEFT_PAD + RIGHT_PAD;
        if area.width <= h_pad {
            return;
        }
        let padded_area = Rect {
            x: area.x + LEFT_PAD,
            y: area.y,
            width: area.width - h_pad,
            height: area.height,
        };

        // Split into header (1) + body + status bar (1)
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // header
                Constraint::Min(0),    // body
                Constraint::Length(1), // status bar
            ])
            .split(padded_area);

        let header_area = chunks[0];
        let body_area = chunks[1];
        let status_area = chunks[2];

        // Update visible height for scroll calculations
        self.visible_height = padded_area.height as usize;

        let available_width = padded_area.width as usize;

        // Calculate column widths dynamically and reduce visible columns to fit
        let mut visible_cols: Vec<usize> = Vec::new();
        let mut col_widths: Vec<u16> = Vec::new();
        let mut used_width: usize = 0;
        let col_gap = 1usize;

        for ci in self.scroll_col..result.columns.len() {
            let header_len = result.columns[ci].chars().count();
            let max_data_len = self
                .formatted_cells
                .iter()
                .skip(self.row_offset)
                .take(100)
                .map(|row| row.get(ci).map_or(0, |s| s.chars().count()))
                .max()
                .unwrap_or(0);
            let data_width = max_data_len.min(40);
            let width = header_len.max(data_width).max(4) as u16;

            let needed = width as usize + if visible_cols.is_empty() { 0 } else { col_gap };
            if used_width + needed > available_width && !visible_cols.is_empty() {
                break;
            }

            used_width += needed;
            visible_cols.push(ci);
            col_widths.push(width);
        }

        if visible_cols.is_empty() {
            return;
        }

        let constraints: Vec<Constraint> =
            col_widths.iter().map(|&w| Constraint::Length(w)).collect();

        // Build header row (all caps)
        let header_cells: Vec<Cell> = visible_cols
            .iter()
            .map(|&ci| {
                Cell::from(result.columns[ci].to_uppercase()).style(
                    Style::default()
                        .fg(theme::accent_dim())
                        .bg(theme::bg_surface())
                        .add_modifier(Modifier::BOLD),
                )
            })
            .collect();
        let header = Row::new(header_cells).height(1);

        // Table bg = BG (dark); cell bg = BG_SURFACE; column_spacing gap reveals the dark BG
        let header_table = Table::new(vec![header], &constraints)
            .column_spacing(1)
            .block(Block::default().style(Style::default().bg(theme::bg())));
        frame.render_widget(header_table, header_area);

        // Build data rows with selected row highlighting
        let visible_row_count = body_area.height as usize;
        let data_rows: Vec<Row> = result
            .rows
            .iter()
            .enumerate()
            .skip(self.row_offset)
            .take(visible_row_count)
            .map(|(abs_i, row)| {
                let is_selected = abs_i == self.selected_row;
                let formatted_row = self.formatted_cells.get(abs_i);
                let cells: Vec<Cell> = visible_cols
                    .iter()
                    .zip(col_widths.iter())
                    .map(|(&ci, &col_w)| {
                        let value = row.get(ci);
                        let is_null = matches!(value, Some(serde_json::Value::Null));
                        let val = formatted_row
                            .and_then(|r| r.get(ci))
                            .map_or("", std::string::String::as_str);
                        let truncated = truncate_with_ellipsis(val, col_w as usize);
                        let bg = if is_selected {
                            theme::bg_highlight()
                        } else {
                            theme::bg_surface()
                        };
                        let style = if is_null {
                            Style::default()
                                .fg(theme::fg_dim())
                                .bg(bg)
                                .add_modifier(Modifier::ITALIC)
                        } else if is_selected {
                            Style::default().fg(theme::fg_bright()).bg(bg)
                        } else {
                            Style::default().fg(theme::fg()).bg(bg)
                        };
                        Cell::from(truncated).style(style)
                    })
                    .collect();
                Row::new(cells).height(1)
            })
            .collect();

        let body_table = Table::new(data_rows, &constraints)
            .column_spacing(1)
            .block(Block::default().style(Style::default().bg(theme::bg())));
        frame.render_widget(body_table, body_area);

        // Status bar
        let total_cols = result.columns.len();
        let visible = visible_cols.len();
        let status = Line::from(vec![
            Span::styled(
                format!(" {} rows", result.row_count),
                Style::default().fg(theme::fg()),
            ),
            Span::styled(
                format!(" | {}ms", result.elapsed_ms),
                Style::default().fg(theme::fg_dim()),
            ),
            Span::styled(
                format!(" | Row {} of {}", self.selected_row + 1, result.row_count,),
                Style::default().fg(theme::fg_dim()),
            ),
            if self.scroll_col > 0 || total_cols > visible {
                Span::styled(
                    format!(
                        " | Cols {}-{} of {}",
                        self.scroll_col + 1,
                        self.scroll_col + visible,
                        total_cols
                    ),
                    Style::default().fg(theme::fg_dim()),
                )
            } else {
                Span::raw("")
            },
            Span::styled(
                " | j/k: Row | h/l: Columns | Ctrl+S: Save | Ctrl+L: Load | Ctrl+N: New chart ",
                Style::default().fg(theme::fg_dim()),
            ),
        ]);

        let status_bar =
            Paragraph::new(status).style(Style::default().bg(theme::bg_surface()).fg(theme::fg()));
        frame.render_widget(status_bar, status_area);
    }
}

fn truncate_with_ellipsis(s: &str, max_width: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_width {
        s.to_string()
    } else if max_width <= 3 {
        s.chars().take(max_width).collect()
    } else {
        let mut result: String = s.chars().take(max_width - 3).collect();
        result.push_str("...");
        result
    }
}
