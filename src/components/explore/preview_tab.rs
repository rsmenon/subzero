#![allow(clippy::cast_possible_truncation, clippy::too_many_lines)]

use std::cell::Cell;

use color_eyre::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell as TableCell, Row, Table as TableWidget};

use crate::action::Action;
use crate::components::Component;
use crate::components::row_detail::RowDetailPopup;
use crate::snowflake::QueryResult;
use crate::theme;

/// Left/right padding width
const LEFT_PAD: u16 = 1;
const RIGHT_PAD: u16 = 1;

pub struct PreviewTab {
    result: Option<QueryResult>,
    selected_row: usize,
    row_offset: usize,
    col_offset: usize,
    visible_height: Cell<usize>,
    loading: bool,
    error: Option<String>,
    pub previewed_table: Option<(String, String, String)>,
    pub row_detail: RowDetailPopup,
}

impl PreviewTab {
    pub fn new() -> Self {
        Self {
            result: None,
            selected_row: 0,
            row_offset: 0,
            col_offset: 0,
            visible_height: Cell::new(20),
            loading: false,
            error: None,
            previewed_table: None,
            row_detail: RowDetailPopup::new(),
        }
    }

    pub fn set_loading(&mut self) {
        self.loading = true;
        self.error = None;
    }

    pub fn set_result(&mut self, result: QueryResult) {
        self.result = Some(result);
        self.selected_row = 0;
        self.row_offset = 0;
        self.col_offset = 0;
        self.loading = false;
        self.error = None;
    }

    pub fn set_error(&mut self, msg: String) {
        self.error = Some(msg);
        self.loading = false;
    }

    pub fn set_previewed_table(&mut self, db: &str, schema: &str, table: &str) {
        self.previewed_table = Some((db.to_string(), schema.to_string(), table.to_string()));
    }

    pub fn has_data_for(&self, db: &str, schema: &str, table: &str) -> bool {
        if let Some((ref pd, ref ps, ref pt)) = self.previewed_table {
            pd == db && ps == schema && pt == table && self.result.is_some()
        } else {
            false
        }
    }

    fn visible_cols() -> usize {
        // Max columns to show at once (may be reduced dynamically during render)
        8
    }

    /// Ensure selected row is visible within the scroll window
    fn adjust_row_offset(&mut self) {
        // Subtract 1 for header row
        let visible = self.visible_height.get().saturating_sub(1);
        if visible == 0 {
            return;
        }
        if self.selected_row < self.row_offset {
            self.row_offset = self.selected_row;
        } else if self.selected_row >= self.row_offset + visible {
            self.row_offset = self.selected_row - visible + 1;
        }
    }

    /// Render the row detail popup over the given area, centered on full screen.
    pub fn render_row_detail(&self, frame: &mut Frame, area: Rect, full_area: Rect) {
        self.row_detail.render(frame, area, full_area);
    }

    /// Return status text for the hint bar (row/column info)
    pub fn status_text(&self) -> String {
        let Some(result) = &self.result else {
            return String::new();
        };
        let total_cols = result.columns.len();
        let max_visible = Self::visible_cols().min(total_cols);
        let max_end = (self.col_offset + max_visible).min(total_cols);
        if total_cols > max_end - self.col_offset {
            format!(
                " {} rows | cols {}-{} of {} (h/l to scroll) | Enter: Row Detail",
                result.row_count,
                self.col_offset + 1,
                max_end,
                total_cols
            )
        } else {
            format!(
                " {} rows | {} columns | Enter: Row Detail",
                result.row_count, total_cols
            )
        }
    }
}

impl Component for PreviewTab {
    fn update(&mut self, _action: Action) -> Result<Option<Action>> {
        Ok(None)
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        // Row detail popup captures all input when visible
        if self.row_detail.visible {
            self.row_detail.handle_key(key);
            return Ok(Some(Action::Render));
        }

        let Some(result) = &self.result else {
            return Ok(None);
        };

        match key.code {
            KeyCode::Enter => {
                if let Some(row) = result.rows.get(self.selected_row) {
                    self.row_detail.show(&result.columns, row);
                }
                Ok(Some(Action::Render))
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected_row < result.rows.len().saturating_sub(1) {
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
            KeyCode::Right | KeyCode::Char('l') => {
                let max_offset = result.columns.len().saturating_sub(Self::visible_cols());
                if self.col_offset < max_offset {
                    self.col_offset += 1;
                }
                Ok(Some(Action::Render))
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.col_offset = self.col_offset.saturating_sub(1);
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
            let paragraph = ratatui::widgets::Paragraph::new("Loading preview...")
                .style(Style::default().fg(theme::fg_dim()))
                .block(block);
            frame.render_widget(paragraph, padded_area);
            return;
        }

        if let Some(ref err) = self.error {
            let paragraph = ratatui::widgets::Paragraph::new(format!("Error: {err}"))
                .style(Style::default().fg(theme::red()))
                .block(block);
            frame.render_widget(paragraph, padded_area);
            return;
        }

        let Some(result) = &self.result else {
            let msg = if self.previewed_table.is_some() {
                "Loading preview..."
            } else {
                "Select a table and switch to Preview tab"
            };
            let paragraph = ratatui::widgets::Paragraph::new(msg)
                .style(Style::default().fg(theme::fg_dim()))
                .block(block);
            frame.render_widget(paragraph, padded_area);
            return;
        };

        if result.columns.is_empty() {
            let paragraph = ratatui::widgets::Paragraph::new("No data")
                .style(Style::default().fg(theme::fg_dim()))
                .block(block);
            frame.render_widget(paragraph, padded_area);
            return;
        }

        let total_cols = result.columns.len();
        let max_visible = Self::visible_cols().min(total_cols);
        let max_end = (self.col_offset + max_visible).min(total_cols);
        let candidate_columns = &result.columns[self.col_offset..max_end];

        // Compute widths, which may reduce the number of visible columns to avoid truncation
        let available_width = padded_area.width as usize;
        let (widths, actual_end) = compute_column_widths(
            candidate_columns,
            &result.rows,
            self.col_offset,
            max_end,
            available_width,
        );
        let visible_columns = &result.columns[self.col_offset..actual_end];

        let header_cells: Vec<TableCell> = visible_columns
            .iter()
            .map(|c| {
                TableCell::from(c.to_uppercase()).style(
                    Style::default()
                        .fg(theme::accent_dim())
                        .bg(theme::bg_surface())
                        .add_modifier(Modifier::BOLD),
                )
            })
            .collect();
        let header = Row::new(header_cells);

        // Update visible height for scroll calculations
        self.visible_height.set(padded_area.height as usize);

        let rows: Vec<Row> = result
            .rows
            .iter()
            .enumerate()
            .skip(self.row_offset)
            .map(|(i, row)| {
                let is_selected = i == self.selected_row;
                let cells: Vec<TableCell> = (self.col_offset..actual_end)
                    .map(|ci| {
                        let bg = if is_selected {
                            theme::bg_highlight()
                        } else {
                            theme::bg_surface()
                        };
                        row.get(ci)
                            .map_or_else(
                                || TableCell::from("").style(Style::default().bg(bg)),
                                |v| match v {
                                    serde_json::Value::Null => TableCell::from("NULL").style(
                                        Style::default()
                                            .fg(theme::fg_dim())
                                            .bg(bg)
                                            .add_modifier(Modifier::ITALIC),
                                    ),
                                    serde_json::Value::String(s) => {
                                        let fg = if is_selected {
                                            theme::fg_bright()
                                        } else {
                                            theme::fg()
                                        };
                                        TableCell::from(s.clone()).style(Style::default().fg(fg).bg(bg))
                                    }
                                    other => {
                                        let fg = if is_selected {
                                            theme::fg_bright()
                                        } else {
                                            theme::fg()
                                        };
                                        TableCell::from(other.to_string())
                                            .style(Style::default().fg(fg).bg(bg))
                                    }
                                },
                            )
                    })
                    .collect();
                Row::new(cells)
            })
            .collect();

        // Table bg = BG (dark); cell bg = BG_SURFACE; column_spacing gap reveals the dark BG
        let block = Block::default()
            .borders(Borders::NONE)
            .style(Style::default().bg(theme::bg()).fg(theme::fg()));

        let table = TableWidget::new(rows, widths)
            .header(header)
            .column_spacing(1)
            .block(block);

        frame.render_widget(table, padded_area);
    }
}

/// Compute column widths based on header names and data content.
/// Never truncates column headers. Reduces visible column count if needed to fit.
/// Returns (widths, `actual_end`) — `actual_end` may be less than `col_end` if columns were dropped.
fn compute_column_widths(
    headers: &[String],
    rows: &[Vec<serde_json::Value>],
    col_start: usize,
    col_end: usize,
    available_width: usize,
) -> (Vec<Constraint>, usize) {
    if col_end <= col_start {
        return (vec![], col_start);
    }

    // Ratatui Table uses 1-char gap between columns
    let col_gap = 1usize;

    // Compute ideal width for each column: max(header, data sample), capped at 40
    let mut ideal_widths: Vec<usize> = Vec::new();
    for (i, ci) in (col_start..col_end).enumerate() {
        let header_len = headers.get(i).map_or(0, |h| h.chars().count());

        let max_data_len = rows
            .iter()
            .take(50)
            .filter_map(|row| row.get(ci))
            .map(|v| match v {
                serde_json::Value::Null => 4,
                serde_json::Value::String(s) => s.chars().count(),
                other => other.to_string().chars().count(),
            })
            .max()
            .unwrap_or(0);

        // At minimum, fit the header; cap content at 40
        let width = header_len.max(max_data_len.min(40)).max(4);
        ideal_widths.push(width);
    }

    // Greedily fit as many columns as possible without truncating headers
    let mut actual_count = ideal_widths.len();
    loop {
        if actual_count == 0 {
            break;
        }
        let total: usize = ideal_widths[..actual_count].iter().sum::<usize>()
            + actual_count.saturating_sub(1) * col_gap;
        if total <= available_width {
            break;
        }
        actual_count -= 1;
    }

    // If even one column doesn't fit, just show it at available width
    if actual_count == 0 && !ideal_widths.is_empty() {
        actual_count = 1;
    }

    let actual_end = col_start + actual_count;
    let widths: Vec<Constraint> = ideal_widths[..actual_count]
        .iter()
        .map(|&w| Constraint::Length(w as u16))
        .collect();

    (widths, actual_end)
}
