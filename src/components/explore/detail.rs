#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_lossless, clippy::too_many_lines)]

use color_eyre::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::action::Action;
use crate::components::Component;
use crate::snowflake::{Column, QueryResult, Table};
use crate::theme;

use super::columns_tab::ColumnsTab;
use super::preview_tab::PreviewTab;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    Columns,
    Preview,
}

pub struct DetailPanel {
    pub active_tab: DetailTab,
    pub breadcrumb: Option<(String, String, String)>, // (db, schema, table)
    pub table_meta: Option<Table>,
    pub columns_tab: ColumnsTab,
    pub preview_tab: PreviewTab,
}

impl DetailPanel {
    pub fn new() -> Self {
        Self {
            active_tab: DetailTab::Columns,
            breadcrumb: None,
            table_meta: None,
            columns_tab: ColumnsTab::new(),
            preview_tab: PreviewTab::new(),
        }
    }

    pub fn set_breadcrumb(&mut self, db: &str, schema: &str, table: &str) {
        self.breadcrumb = Some((db.to_string(), schema.to_string(), table.to_string()));
    }

    pub fn set_table_meta(&mut self, table: Option<Table>) {
        self.table_meta = table;
    }

    pub fn set_columns(&mut self, columns: Vec<Column>) {
        self.columns_tab.set_columns(columns);
    }

    pub fn set_preview(&mut self, result: QueryResult) {
        self.preview_tab.set_result(result);
    }

    pub fn set_columns_loading(&mut self) {
        self.columns_tab.set_loading();
    }

    pub fn set_preview_loading(&mut self) {
        self.preview_tab.set_loading();
    }

    pub fn set_columns_error(&mut self, msg: String) {
        self.columns_tab.set_error(msg);
    }

    pub fn set_preview_error(&mut self, msg: String) {
        self.preview_tab.set_error(msg);
    }

    /// Render the table stats section showing key metadata.
    /// Layout: Row 1 = Created, Type, Rows, Size, Auto Clustering
    ///         Row 2 = Clustering (full width)
    fn render_stats(&self, frame: &mut Frame, area: Rect) {
        if area.height == 0 || area.width < 4 {
            return;
        }

        let Some(meta) = &self.table_meta else {
            let bg =
                Paragraph::new("").style(Style::default().bg(theme::bg_surface()).fg(theme::fg()));
            frame.render_widget(bg, area);
            return;
        };

        let label_style = Style::default().fg(theme::fg_dim());
        let value_style = Style::default().fg(theme::fg());

        // Row 1: Created, Type, Rows, Size, Auto Clustering — inline with separators
        let mut row1_spans: Vec<Span> =
            vec![Span::styled(" ", Style::default().bg(theme::bg_surface()))];
        let sep = Span::styled("  ", Style::default().bg(theme::bg_surface()));

        if let Some(ref created) = meta.created {
            let display = if created.len() > 10 {
                created.chars().take(10).collect::<String>()
            } else {
                created.clone()
            };
            row1_spans.push(Span::styled("Created: ", label_style));
            row1_spans.push(Span::styled(display, value_style));
            row1_spans.push(sep.clone());
        }

        row1_spans.push(Span::styled("Type: ", label_style));
        row1_spans.push(Span::styled(meta.kind.clone(), value_style));
        row1_spans.push(sep.clone());

        if let Some(rc) = meta.row_count {
            row1_spans.push(Span::styled("Rows: ", label_style));
            row1_spans.push(Span::styled(format_rows_human(rc), value_style));
            row1_spans.push(sep.clone());
        }

        if let Some(bytes) = meta.bytes {
            row1_spans.push(Span::styled("Size: ", label_style));
            row1_spans.push(Span::styled(format_bytes(bytes), value_style));
            row1_spans.push(sep.clone());
        }

        if let Some(ac) = meta.auto_clustering {
            row1_spans.push(Span::styled("Auto Clustering: ", label_style));
            row1_spans.push(Span::styled(
                if ac { "ON" } else { "OFF" }.to_string(),
                value_style,
            ));
        }

        let mut lines: Vec<Line> = vec![Line::from(row1_spans)];

        // Row 2: Clustering key (can extend nearly full width)
        if let Some(ref ck) = meta.clustering_key {
            let max_ck_width = (area.width as usize).saturating_sub(16); // "  Clustering: " + margin
            let display = truncate_to_width(ck, max_ck_width);
            lines.push(Line::from(vec![
                Span::styled(" ", Style::default().bg(theme::bg_surface())),
                Span::styled("Clustering: ", label_style),
                Span::styled(display, value_style),
            ]));
        }

        let paragraph =
            Paragraph::new(lines).style(Style::default().bg(theme::bg_surface()).fg(theme::fg()));
        frame.render_widget(paragraph, area);
    }

    fn render_tab_bar(&self, frame: &mut Frame, area: Rect, _focused: bool) {
        let tabs: Vec<(&str, DetailTab)> = vec![
            ("Columns", DetailTab::Columns),
            ("Preview", DetailTab::Preview),
        ];

        let spans: Vec<Span> = tabs
            .iter()
            .enumerate()
            .flat_map(|(i, (label, tab))| {
                let style = if *tab == self.active_tab {
                    Style::default()
                        .fg(theme::fg_bright())
                        .bg(theme::tab_active_bg())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(theme::fg_dim())
                        .bg(theme::tab_inactive_bg())
                };
                let mut result = vec![Span::styled(format!(" {label} "), style)];
                if i < tabs.len() - 1 {
                    result.push(Span::styled(" ", Style::default().bg(theme::bg_surface())));
                }
                result
            })
            .collect();

        let mut line_spans = vec![Span::styled(" ", Style::default().bg(theme::bg_surface()))];
        line_spans.extend(spans);
        let line = Line::from(line_spans);
        let paragraph =
            Paragraph::new(line).style(Style::default().bg(theme::bg_surface()).fg(theme::fg()));
        frame.render_widget(paragraph, area);
    }
}

impl Component for DetailPanel {
    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match &action {
            Action::ColumnsLoaded { columns, .. } => {
                self.set_columns(columns.clone());
                Ok(Some(Action::Render))
            }
            Action::PreviewLoaded {
                db,
                schema,
                table,
                result,
            } => {
                self.preview_tab.set_previewed_table(db, schema, table);
                self.set_preview(result.clone());
                Ok(Some(Action::Render))
            }
            Action::ColumnsError { error, .. } => {
                self.set_columns_error(error.clone());
                Ok(Some(Action::Render))
            }
            Action::PreviewError { error, .. } => {
                self.set_preview_error(error.clone());
                Ok(Some(Action::Render))
            }
            _ => Ok(None),
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        // '/' activates column search when on Columns tab
        if key.code == KeyCode::Char('/') && self.active_tab == DetailTab::Columns {
            self.columns_tab.activate_search();
            return Ok(Some(Action::Render));
        }

        // If column search is active, route all keys there
        if self.columns_tab.search_active() {
            return self.columns_tab.handle_key(key);
        }

        match key.code {
            // Letter-based tab switching: C for Columns, P for Preview
            KeyCode::Char('c' | 'C' | '1') => {
                self.active_tab = DetailTab::Columns;
                Ok(Some(Action::Render))
            }
            KeyCode::Char('p' | 'P' | '2') => {
                self.active_tab = DetailTab::Preview;
                // Auto-fire preview load if a table is selected and no data for it
                if let Some((ref db, ref schema, ref table)) = self.breadcrumb
                    && !self.preview_tab.has_data_for(db, schema, table)
                {
                    return Ok(Some(Action::LoadPreview {
                        db: db.clone(),
                        schema: schema.clone(),
                        table: table.clone(),
                    }));
                }
                Ok(Some(Action::Render))
            }
            // R in detail pane refreshes columns for the currently selected table (bypasses cache)
            KeyCode::Char('R') => {
                if let Some((ref db, ref schema, ref table)) = self.breadcrumb {
                    Ok(Some(Action::RefreshColumns {
                        db: db.clone(),
                        schema: schema.clone(),
                        table: table.clone(),
                    }))
                } else {
                    Ok(None)
                }
            }
            _ => match self.active_tab {
                DetailTab::Columns => self.columns_tab.handle_key(key),
                DetailTab::Preview => self.preview_tab.handle_key(key),
            },
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        self.render_with_focus(frame, area, false);
    }
}

impl DetailPanel {
    pub fn render_with_focus(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        // Split into two panes: Quick Info (top, not interactive) + Details (bottom, interactive)
        // Quick Info height: breadcrumb (1) + stats (1 or 2) + padding = ~4 lines
        let stats_height: u16 = if let Some(meta) = &self.table_meta {
            if meta.clustering_key.is_some() { 2 } else { 1 }
        } else {
            0
        };
        // Quick Info: top padding (1) + stats rows + 1 bottom padding
        let quick_info_height = 1 + stats_height + 1;

        let pane_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(quick_info_height + 2), // +2 for borders
                Constraint::Min(4),
            ])
            .split(area);

        // --- Quick Info Pane (not focusable, always uses BORDER) ---
        // Use breadcrumb as the pane title
        let qi_title = if let Some((ref db, ref schema, ref table)) = self.breadcrumb {
            Line::from(vec![
                Span::styled(" ", Style::default()),
                Span::styled(db.as_str(), Style::default().fg(theme::fg_dim())),
                Span::styled(" \u{203a} ", Style::default().fg(theme::fg_mid())),
                Span::styled(schema.as_str(), Style::default().fg(theme::accent())),
                Span::styled(" \u{203a} ", Style::default().fg(theme::fg_mid())),
                Span::styled(
                    table.as_str(),
                    Style::default()
                        .fg(theme::accent_dim())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" ", Style::default()),
            ])
        } else {
            Line::from(Span::styled(
                " No table selected ",
                Style::default().fg(theme::fg_dim()),
            ))
        };

        let qi_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::border()))
            .title(qi_title)
            .style(Style::default().bg(theme::bg_surface()).fg(theme::fg()));

        let qi_inner = qi_block.inner(pane_chunks[0]);
        frame.render_widget(qi_block, pane_chunks[0]);

        if qi_inner.height >= 1 && qi_inner.width >= 4 {
            let qi_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),            // top padding
                    Constraint::Length(stats_height), // stats
                    Constraint::Min(0),               // remaining padding
                ])
                .split(qi_inner);

            self.render_stats(frame, qi_chunks[1]);
        }

        // --- Details Pane (focusable) ---
        let border_color = if focused {
            theme::border_focus()
        } else {
            theme::border()
        };
        let detail_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(" Details ")
            .title_style(Style::default().fg(theme::accent_dim()))
            .style(Style::default().bg(theme::bg_surface()).fg(theme::fg()));

        let detail_inner = detail_block.inner(pane_chunks[1]);
        frame.render_widget(detail_block, pane_chunks[1]);

        if detail_inner.height < 3 || detail_inner.width < 4 {
            return;
        }

        let detail_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // top padding
                Constraint::Length(1), // tab bar
                Constraint::Length(1), // separator
                Constraint::Min(0),    // tab content
                Constraint::Length(1), // hint bar
            ])
            .split(detail_inner);

        self.render_tab_bar(frame, detail_chunks[1], focused);

        // Separator line between tab bar and content
        let sep = "\u{2500}".repeat(detail_chunks[2].width as usize);
        let sep_line = Paragraph::new(Line::from(Span::styled(
            sep,
            Style::default().fg(theme::border()),
        )))
        .style(Style::default().bg(theme::bg_surface()).fg(theme::fg()));
        frame.render_widget(sep_line, detail_chunks[2]);

        match self.active_tab {
            DetailTab::Columns => self.columns_tab.render(frame, detail_chunks[3]),
            DetailTab::Preview => self.preview_tab.render(frame, detail_chunks[3]),
        }

        // Render row detail popup on top of the detail pane, centered on full screen
        if self.active_tab == DetailTab::Preview {
            self.preview_tab
                .render_row_detail(frame, pane_chunks[1], frame.area());
        }

        // Hint bar
        let accent = Style::default().fg(theme::accent_dim());
        let dim = Style::default().fg(theme::fg_dim());
        let hint_line: Line = if focused {
            if self.columns_tab.search_active() {
                Line::from(vec![
                    Span::styled(" Type to filter", dim),
                    Span::styled("  ", dim),
                    Span::styled("Esc", accent),
                    Span::styled(": Close search", dim),
                ])
            } else if self.active_tab == DetailTab::Preview {
                let status = self.preview_tab.status_text();
                if status.is_empty() {
                    Line::from(vec![
                        Span::styled(" C", accent),
                        Span::styled(": Columns  ", dim),
                        Span::styled("S-Tab", accent),
                        Span::styled(": Tree", dim),
                    ])
                } else {
                    Line::from(vec![
                        Span::styled(format!(" {status}"), dim),
                        Span::styled("  ", dim),
                        Span::styled("C", accent),
                        Span::styled(": Columns", dim),
                    ])
                }
            } else {
                Line::from(vec![
                    Span::styled(" P", accent),
                    Span::styled(": Preview  ", dim),
                    Span::styled("/", accent),
                    Span::styled(": Search  ", dim),
                    Span::styled("R", accent),
                    Span::styled(": Refresh  ", dim),
                    Span::styled("S-Tab", accent),
                    Span::styled(": Tree", dim),
                ])
            }
        } else {
            Line::from("")
        };
        let hint_paragraph =
            Paragraph::new(hint_line).style(Style::default().bg(theme::bg_surface()).fg(theme::fg()));
        frame.render_widget(hint_paragraph, detail_chunks[4]);
    }
}

/// Format a byte count into human-readable form (kB, MB, GB, TB)
fn format_bytes(bytes: i64) -> String {
    let abs = bytes.unsigned_abs() as f64;
    if abs < 1024.0 {
        format!("{bytes} B")
    } else if abs < 1024.0 * 1024.0 {
        format!("{:.1} KB", abs / 1024.0)
    } else if abs < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MB", abs / (1024.0 * 1024.0))
    } else if abs < 1024.0 * 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} GB", abs / (1024.0 * 1024.0 * 1024.0))
    } else {
        format!("{:.1} TB", abs / (1024.0 * 1024.0 * 1024.0 * 1024.0))
    }
}

/// Format a row count in human-readable form (k, M, B, T)
fn format_rows_human(n: i64) -> String {
    let abs = n.unsigned_abs() as f64;
    let sign = if n < 0 { "-" } else { "" };
    if abs < 1_000.0 {
        format!("{}{}", sign, n.unsigned_abs())
    } else if abs < 1_000_000.0 {
        format!("{}{:.1}k", sign, abs / 1_000.0)
    } else if abs < 1_000_000_000.0 {
        format!("{}{:.1}M", sign, abs / 1_000_000.0)
    } else if abs < 1_000_000_000_000.0 {
        format!("{}{:.1}B", sign, abs / 1_000_000_000.0)
    } else {
        format!("{}{:.1}T", sign, abs / 1_000_000_000_000.0)
    }
}

/// Truncate a string to fit within a given character width, adding "..." if truncated
fn truncate_to_width(s: &str, max_width: usize) -> String {
    if s.chars().count() <= max_width {
        s.to_string()
    } else if max_width > 3 {
        let truncated: String = s.chars().take(max_width - 3).collect();
        format!("{truncated}...")
    } else {
        s.chars().take(max_width).collect()
    }
}
