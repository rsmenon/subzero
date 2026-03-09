#![allow(clippy::cast_possible_truncation)]

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table};

use crate::theme;

use super::config::{Aggregation, ChartConfig, ChartType, SortOrder};
use super::prepare::detect_all_column_types;
use super::types::ColumnType;
use crate::snowflake::QueryResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsFocus {
    ChartType,
    XColumn,
    YColumn,
    SortOrder,
    GroupBy,
    Aggregation,
    Generate,
}

impl SettingsFocus {
    pub fn next(self) -> Self {
        match self {
            SettingsFocus::ChartType => SettingsFocus::XColumn,
            SettingsFocus::XColumn => SettingsFocus::YColumn,
            SettingsFocus::YColumn => SettingsFocus::SortOrder,
            SettingsFocus::SortOrder => SettingsFocus::GroupBy,
            SettingsFocus::GroupBy => SettingsFocus::Aggregation,
            SettingsFocus::Aggregation | SettingsFocus::Generate => SettingsFocus::Generate,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            SettingsFocus::ChartType | SettingsFocus::XColumn => SettingsFocus::ChartType,
            SettingsFocus::YColumn => SettingsFocus::XColumn,
            SettingsFocus::SortOrder => SettingsFocus::YColumn,
            SettingsFocus::GroupBy => SettingsFocus::SortOrder,
            SettingsFocus::Aggregation => SettingsFocus::GroupBy,
            SettingsFocus::Generate => SettingsFocus::Aggregation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupField {
    ChartType,
    XColumn,
    YColumn,
    SortOrder,
    GroupBy,
    Aggregation,
}

pub struct ChartSettings {
    pub focus: SettingsFocus,
    pub popup_open: Option<PopupField>,
    pub popup_index: usize,
    /// Cached column types from current result
    column_types: Vec<(String, ColumnType)>,
}

impl ChartSettings {
    pub fn new() -> Self {
        Self {
            focus: SettingsFocus::ChartType,
            popup_open: None,
            popup_index: 0,
            column_types: Vec::new(),
        }
    }

    pub fn update_column_types(&mut self, result: &QueryResult) {
        self.column_types = detect_all_column_types(result);
    }

    pub fn clear_column_types(&mut self) {
        self.column_types.clear();
    }

    /// Get all columns with their types
    fn all_columns(&self) -> &[(String, ColumnType)] {
        &self.column_types
    }

    /// Get popup items for a field
    fn popup_items(&self, field: PopupField) -> Vec<(String, String)> {
        match field {
            PopupField::ChartType => ChartType::all()
                .iter()
                .map(|t| (t.label().to_string(), String::new()))
                .collect(),
            PopupField::XColumn | PopupField::YColumn => self
                .all_columns()
                .iter()
                .map(|(name, ct)| (name.clone(), ct.short_label().to_string()))
                .collect(),
            PopupField::SortOrder => SortOrder::all()
                .iter()
                .map(|s| (s.label().to_string(), String::new()))
                .collect(),
            PopupField::GroupBy => {
                let mut items = vec![("(none)".to_string(), String::new())];
                items.extend(
                    self.all_columns()
                        .iter()
                        .map(|(name, ct)| (name.clone(), ct.short_label().to_string())),
                );
                items
            }
            PopupField::Aggregation => Aggregation::all()
                .iter()
                .map(|a| (a.label().to_string(), String::new()))
                .collect(),
        }
    }

    /// Handle key input. Returns:
    /// - Some(true) if Generate was activated
    /// - Some(false) if key was consumed but Generate not activated
    /// - None if Esc should bubble up (unfocus settings)
    pub fn handle_key(&mut self, key: KeyEvent, config: &mut ChartConfig) -> Option<bool> {
        // If a popup is open, handle popup keys
        if let Some(field) = self.popup_open {
            return Some(self.handle_popup_key(key, config, field));
        }

        match key.code {
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Tab => {
                self.focus = self.focus.next();
                Some(false)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.focus = self.focus.prev();
                Some(false)
            }
            KeyCode::Enter | KeyCode::Char(' ' | 'l') | KeyCode::Right => {
                Some(self.activate_field(config))
            }
            KeyCode::Char('h') | KeyCode::Left => {
                // For non-popup fields, no-op
                Some(false)
            }
            KeyCode::Esc => {
                None // bubble up
            }
            _ => Some(false),
        }
    }

    fn activate_field(&mut self, config: &ChartConfig) -> bool {
        match self.focus {
            SettingsFocus::ChartType => {
                self.open_popup(PopupField::ChartType, config);
                false
            }
            SettingsFocus::XColumn => {
                self.open_popup(PopupField::XColumn, config);
                false
            }
            SettingsFocus::YColumn => {
                self.open_popup(PopupField::YColumn, config);
                false
            }
            SettingsFocus::SortOrder => {
                self.open_popup(PopupField::SortOrder, config);
                false
            }
            SettingsFocus::GroupBy => {
                self.open_popup(PopupField::GroupBy, config);
                false
            }
            SettingsFocus::Aggregation => {
                self.open_popup(PopupField::Aggregation, config);
                false
            }
            SettingsFocus::Generate => true,
        }
    }

    fn open_popup(&mut self, field: PopupField, config: &ChartConfig) {
        let items = self.popup_items(field);
        if items.is_empty() {
            return;
        }
        self.popup_open = Some(field);
        // Set popup_index to current selection
        self.popup_index = match field {
            PopupField::ChartType => ChartType::all()
                .iter()
                .position(|t| *t == config.chart_type)
                .unwrap_or(0),
            PopupField::XColumn => {
                if let Some(ref x) = config.x_column {
                    self.all_columns()
                        .iter()
                        .position(|(name, _)| name == x)
                        .unwrap_or(0)
                } else {
                    0
                }
            }
            PopupField::YColumn => {
                if let Some(ref y) = config.y_column {
                    self.all_columns()
                        .iter()
                        .position(|(name, _)| name == y)
                        .unwrap_or(0)
                } else {
                    0
                }
            }
            PopupField::SortOrder => SortOrder::all()
                .iter()
                .position(|s| *s == config.sort_order)
                .unwrap_or(0),
            PopupField::GroupBy => {
                if let Some(ref g) = config.group_by {
                    self.all_columns()
                        .iter()
                        .position(|(name, _)| name == g)
                        .map_or(0, |i| i + 1)
                } else {
                    0
                }
            }
            PopupField::Aggregation => Aggregation::all()
                .iter()
                .position(|a| *a == config.aggregation)
                .unwrap_or(0),
        };
    }

    fn handle_popup_key(
        &mut self,
        key: KeyEvent,
        config: &mut ChartConfig,
        field: PopupField,
    ) -> bool {
        let items = self.popup_items(field);
        let count = items.len();
        if count == 0 {
            self.popup_open = None;
            return false;
        }

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if self.popup_index + 1 < count {
                    self.popup_index += 1;
                }
                false
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.popup_index = self.popup_index.saturating_sub(1);
                false
            }
            KeyCode::Enter => {
                self.apply_popup_selection(config, field);
                self.popup_open = None;
                false
            }
            KeyCode::Esc => {
                self.popup_open = None;
                false
            }
            _ => false,
        }
    }

    fn apply_popup_selection(&self, config: &mut ChartConfig, field: PopupField) {
        match field {
            PopupField::ChartType => {
                let types = ChartType::all();
                if let Some(t) = types.get(self.popup_index) {
                    config.chart_type = t.clone();
                }
            }
            PopupField::XColumn => {
                if let Some((name, _)) = self.all_columns().get(self.popup_index) {
                    config.x_column = Some(name.clone());
                }
            }
            PopupField::YColumn => {
                if let Some((name, _)) = self.all_columns().get(self.popup_index) {
                    config.y_column = Some(name.clone());
                }
            }
            PopupField::SortOrder => {
                let sorts = SortOrder::all();
                if let Some(s) = sorts.get(self.popup_index) {
                    config.sort_order = s.clone();
                }
            }
            PopupField::GroupBy => {
                if self.popup_index == 0 {
                    config.group_by = None;
                } else if let Some((name, _)) = self.all_columns().get(self.popup_index - 1) {
                    config.group_by = Some(name.clone());
                }
            }
            PopupField::Aggregation => {
                let aggs = Aggregation::all();
                if let Some(a) = aggs.get(self.popup_index) {
                    config.aggregation = a.clone();
                }
            }
        }
    }

    /// Render the compact settings panel (label: value on same line)
    pub fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        config: &ChartConfig,
        focused: bool,
        has_data: bool,
    ) {
        let border_color = if focused {
            theme::border_focus()
        } else {
            theme::border()
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(" Settings ")
            .title_style(Style::default().fg(theme::accent_dim()))
            .style(Style::default().bg(theme::bg_surface()).fg(theme::fg()));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if !has_data {
            let msg = Paragraph::new("Run a query first to configure charts.")
                .style(Style::default().fg(theme::fg_dim()).bg(theme::bg_surface()))
                .alignment(Alignment::Center);
            frame.render_widget(msg, inner);
            return;
        }

        if inner.height < 7 || inner.width < 10 {
            return;
        }

        let x = inner.x + 1;
        let w = inner.width.saturating_sub(2);
        let mut y = inner.y;

        // Each field: "Label: Value" on one line
        // 1. Chart Type
        y = Self::render_compact_field(
            frame,
            x,
            y,
            w,
            "Type",
            config.chart_type.label(),
            self.focus == SettingsFocus::ChartType,
        );

        // 2. X Axis
        y = Self::render_compact_field(
            frame,
            x,
            y,
            w,
            "X Axis",
            config.x_column.as_deref().unwrap_or("(select)"),
            self.focus == SettingsFocus::XColumn,
        );

        // 3. Y Axis
        y = Self::render_compact_field(
            frame,
            x,
            y,
            w,
            "Y Axis",
            config.y_column.as_deref().unwrap_or("(select)"),
            self.focus == SettingsFocus::YColumn,
        );

        // 4. Sort
        y = Self::render_compact_field(
            frame,
            x,
            y,
            w,
            "Sort",
            config.sort_order.label(),
            self.focus == SettingsFocus::SortOrder,
        );

        // 5. Group By
        y = Self::render_compact_field(
            frame,
            x,
            y,
            w,
            "Group",
            config.group_by.as_deref().unwrap_or("(none)"),
            self.focus == SettingsFocus::GroupBy,
        );

        // 6. Aggregation
        y = Self::render_compact_field(
            frame,
            x,
            y,
            w,
            "Agg",
            config.aggregation.label(),
            self.focus == SettingsFocus::Aggregation,
        );

        // Blank line
        y += 1;

        // 6. Generate button
        if y < inner.y + inner.height {
            let is_focused = self.focus == SettingsFocus::Generate;
            let btn_style = if is_focused {
                Style::default()
                    .fg(theme::fg_bright())
                    .bg(theme::accent_dim())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::fg()).bg(theme::bg_highlight())
            };
            let btn_text = " Generate ";
            let line = Line::from(Span::styled(btn_text, btn_style));
            let btn_area = Rect::new(x, y, w.min(btn_text.len() as u16), 1);
            frame.render_widget(
                Paragraph::new(line).style(Style::default().bg(theme::bg_surface())),
                btn_area,
            );
        }
    }

    /// Render a single compact field: "Label: Value" on one line
    #[allow(clippy::too_many_arguments)]
    fn render_compact_field(
        frame: &mut Frame,
        x: u16,
        y: u16,
        w: u16,
        label: &str,
        value: &str,
        focused: bool,
    ) -> u16 {
        let label_width = 6; // Fixed label column width
        let colon = ": ";
        let prefix_len = label_width + colon.len();
        let val_max_len = (w as usize).saturating_sub(prefix_len);

        let label_style = if focused {
            Style::default().fg(theme::accent())
        } else {
            Style::default().fg(theme::fg_dim())
        };

        let val_style = if focused {
            Style::default()
                .fg(theme::fg_bright())
                .bg(theme::bg_highlight())
        } else {
            Style::default().fg(theme::fg())
        };

        // Pad label to fixed width
        let padded_label = format!("{label:<label_width$}");

        let truncated_val: String = value.chars().take(val_max_len).collect();
        let arrow = if focused { " \u{25B6}" } else { "" };
        let val_with_arrow: String = if truncated_val.chars().count() + arrow.len() > val_max_len {
            let t: String = truncated_val
                .chars()
                .take(val_max_len.saturating_sub(arrow.len()))
                .collect();
            format!("{t}{arrow}")
        } else {
            format!("{truncated_val}{arrow}")
        };

        let spans = vec![
            Span::styled(padded_label, label_style),
            Span::styled(colon, Style::default().fg(theme::fg_dim())),
            Span::styled(val_with_arrow, val_style),
        ];

        let line = Line::from(spans);
        let field_area = Rect::new(x, y, w, 1);
        frame.render_widget(
            Paragraph::new(line).style(Style::default().bg(theme::bg_surface())),
            field_area,
        );

        y + 1
    }

    /// Render the popup overlay on top of everything.
    /// This should be called AFTER all other rendering so it appears on top.
    pub fn render_popup(&self, frame: &mut Frame, area: Rect, _config: &ChartConfig) {
        let Some(field) = self.popup_open else {
            return;
        };

        let title = match field {
            PopupField::ChartType => "Chart Type",
            PopupField::XColumn => "X Axis",
            PopupField::YColumn => "Y Axis",
            PopupField::SortOrder => "Sort",
            PopupField::GroupBy => "Group By",
            PopupField::Aggregation => "Aggregation",
        };

        let items = self.popup_items(field);
        if items.is_empty() {
            return;
        }

        // Calculate popup dimensions
        let has_type_col = items.iter().any(|(_, t)| !t.is_empty());
        let max_name_len = items
            .iter()
            .map(|(name, _)| name.chars().count())
            .max()
            .unwrap_or(10);
        let max_type_len = if has_type_col {
            items
                .iter()
                .map(|(_, t)| t.chars().count())
                .max()
                .unwrap_or(0)
        } else {
            0
        };

        let content_width = if has_type_col {
            max_name_len + max_type_len + 5 // padding + separator
        } else {
            max_name_len + 4
        };
        let popup_width = (content_width as u16 + 4)
            .min(area.width.saturating_sub(4))
            .max(20);
        let popup_height = (items.len() as u16 + 4)
            .min(area.height.saturating_sub(4))
            .max(5);

        // Center the popup
        let popup_area = centered_rect(popup_width, popup_height, area);

        // Clear background
        frame.render_widget(Clear, popup_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::dialog_border()))
            .title(format!(" {title} "))
            .title_style(Style::default().fg(theme::accent_dim()))
            .style(Style::default().bg(theme::dialog_bg()).fg(theme::fg()));

        let inner = block.inner(popup_area);
        frame.render_widget(block, popup_area);

        if inner.height < 1 || inner.width < 4 {
            return;
        }

        // Render items as a table
        let visible_items = (inner.height as usize).min(items.len());

        // Determine scroll offset to keep selected item visible
        let scroll_offset = if self.popup_index >= visible_items {
            self.popup_index - visible_items + 1
        } else {
            0
        };

        let constraints = if has_type_col {
            vec![
                Constraint::Min(4),
                Constraint::Length(max_type_len as u16 + 2),
            ]
        } else {
            vec![Constraint::Min(4)]
        };

        let rows: Vec<Row> = items
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(visible_items)
            .map(|(i, (name, type_str))| {
                let is_selected = i == self.popup_index;
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

                let name_cell =
                    Cell::from(format!(" {name}")).style(Style::default().fg(fg).bg(bg));

                if has_type_col {
                    let type_style = Style::default().fg(theme::fg_dim()).bg(bg);
                    let type_cell = Cell::from(type_str.clone()).style(type_style);
                    Row::new(vec![name_cell, type_cell])
                } else {
                    Row::new(vec![name_cell])
                }
            })
            .collect();

        let table = Table::new(rows, &constraints)
            .column_spacing(1)
            .block(Block::default().style(Style::default().bg(theme::dialog_bg())));

        frame.render_widget(table, inner);
    }
}

/// Create a centered rect within the given area
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .split(area);
    let horizontal = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .split(vertical[0]);
    horizontal[0]
}
