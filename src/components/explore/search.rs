#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_lossless, clippy::too_many_lines)]

use color_eyre::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

use crate::action::Action;
use crate::components::Component;
use crate::theme;

/// An item in the fuzzy search index
#[derive(Debug, Clone)]
pub struct SearchEntry {
    /// Display text like "DB / SCHEMA / TABLE"
    pub display: String,
    /// Tree path for navigating: (db, schema, `table_or_empty`)
    pub db: String,
    pub schema: String,
    pub table: Option<String>,
    pub is_view: bool,
    /// Pre-computed lowercased fields for search (avoids per-keystroke allocation)
    db_lower: String,
    schema_lower: String,
    table_lower: Option<String>,
}

pub struct FuzzySearch {
    pub active: bool,
    pub input: String,
    pub entries: Vec<SearchEntry>,
    pub results: Vec<usize>, // indices into entries
    pub selected: usize,
    pub scroll_offset: usize,
}

impl FuzzySearch {
    pub fn new() -> Self {
        Self {
            active: false,
            input: String::new(),
            entries: Vec::new(),
            results: Vec::new(),
            selected: 0,
            scroll_offset: 0,
        }
    }

    pub fn activate(&mut self) {
        self.active = true;
        self.input.clear();
        self.selected = 0;
        self.scroll_offset = 0;
        // Show all entries initially
        self.results = (0..self.entries.len()).collect();
    }

    pub fn deactivate(&mut self) {
        self.active = false;
        self.input.clear();
        self.results.clear();
        self.selected = 0;
        self.scroll_offset = 0;
    }

    /// Rebuild search entries from catalog data
    pub fn rebuild_entries(
        &mut self,
        databases: &[crate::snowflake::Database],
        schemas: &[crate::snowflake::Schema],
        tables: &[crate::snowflake::Table],
    ) {
        self.entries.clear();

        for db in databases {
            self.entries.push(SearchEntry {
                display: db.name.clone(),
                db_lower: db.name.to_lowercase(),
                db: db.name.clone(),
                schema: String::new(),
                schema_lower: String::new(),
                table: None,
                table_lower: None,
                is_view: false,
            });
        }

        for schema in schemas {
            self.entries.push(SearchEntry {
                display: format!("{} / {}", schema.database, schema.name),
                db_lower: schema.database.to_lowercase(),
                db: schema.database.clone(),
                schema_lower: schema.name.to_lowercase(),
                schema: schema.name.clone(),
                table: None,
                table_lower: None,
                is_view: false,
            });
        }

        for table in tables {
            let is_view = table.kind.to_uppercase() == "VIEW";
            let suffix = if is_view { " (V)" } else { "" };
            self.entries.push(SearchEntry {
                display: format!(
                    "{} / {} / {}{}",
                    table.database, table.schema, table.name, suffix
                ),
                db_lower: table.database.to_lowercase(),
                db: table.database.clone(),
                schema_lower: table.schema.to_lowercase(),
                schema: table.schema.clone(),
                table_lower: Some(table.name.to_lowercase()),
                table: Some(table.name.clone()),
                is_view,
            });
        }

        self.results = (0..self.entries.len()).collect();
    }

    /// Substring match: check if query appears as a contiguous substring in the entry's
    /// display name, table name, schema name, or database name
    fn substring_match(query: &str, entry: &SearchEntry) -> bool {
        if query.is_empty() {
            return true;
        }
        let query_lower = query.to_lowercase();
        // Check against pre-computed lowercased components
        if entry.db_lower.contains(&query_lower) {
            return true;
        }
        if !entry.schema_lower.is_empty() && entry.schema_lower.contains(&query_lower) {
            return true;
        }
        if let Some(ref table_lower) = entry.table_lower
            && table_lower.contains(&query_lower)
        {
            return true;
        }
        false
    }

    fn update_results(&mut self) {
        self.results = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| Self::substring_match(&self.input, entry))
            .map(|(i, _)| i)
            .collect();
        self.selected = 0;
        self.scroll_offset = 0;
    }

    /// Ensure the selected item is within the visible scroll window.
    fn adjust_scroll(&mut self) {
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + 20 {
            self.scroll_offset = self.selected.saturating_sub(19);
        }
    }

    /// Get the currently selected search entry
    pub fn selected_entry(&self) -> Option<&SearchEntry> {
        self.results
            .get(self.selected)
            .and_then(|&idx| self.entries.get(idx))
    }

    /// Build tree-structured display lines from results.
    /// Returns (lines, `result_index_per_line`) where `result_index_per_line` maps each
    /// rendered line to the result index it corresponds to (or None for parent headers).
    #[allow(clippy::type_complexity)]
    fn build_tree_lines(&self) -> (Vec<TreeLine>, Vec<Option<usize>>) {
        use std::collections::BTreeMap;

        // Group results by db > schema
        // We need to preserve insertion order within groups, so use BTreeMap for sorted display
        let mut db_schemas: BTreeMap<String, BTreeMap<String, Vec<(usize, &SearchEntry)>>> =
            BTreeMap::new();

        // Also track db-only and schema-only entries
        let mut db_only: BTreeMap<String, Vec<(usize, &SearchEntry)>> = BTreeMap::new();
        let mut schema_only: BTreeMap<String, BTreeMap<String, Vec<(usize, &SearchEntry)>>> =
            BTreeMap::new();

        for (result_idx, &entry_idx) in self.results.iter().enumerate() {
            let entry = &self.entries[entry_idx];
            if entry.table.is_some() {
                // Table/view entry
                db_schemas
                    .entry(entry.db.clone())
                    .or_default()
                    .entry(entry.schema.clone())
                    .or_default()
                    .push((result_idx, entry));
            } else if !entry.schema.is_empty() {
                // Schema-level entry
                schema_only
                    .entry(entry.db.clone())
                    .or_default()
                    .entry(entry.schema.clone())
                    .or_default()
                    .push((result_idx, entry));
            } else {
                // Database-level entry
                db_only
                    .entry(entry.db.clone())
                    .or_default()
                    .push((result_idx, entry));
            }
        }

        let mut lines: Vec<TreeLine> = Vec::new();
        let mut line_to_result: Vec<Option<usize>> = Vec::new();

        // Collect all database names that appear
        let mut all_dbs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for db in db_schemas.keys() {
            all_dbs.insert(db.clone());
        }
        for db in db_only.keys() {
            all_dbs.insert(db.clone());
        }
        for db in schema_only.keys() {
            all_dbs.insert(db.clone());
        }

        for db_name in &all_dbs {
            let has_table_children = db_schemas.contains_key(db_name);
            let has_schema_children = schema_only.contains_key(db_name);

            // Check if there's a db-only result for this db
            let db_result_idx = db_only
                .get(db_name)
                .and_then(|v| v.first())
                .map(|(idx, _)| *idx);

            if has_table_children || has_schema_children {
                // Render as a parent header
                lines.push(TreeLine {
                    indent: 0,
                    text: db_name.clone(),
                    is_header: true,
                });
                line_to_result.push(db_result_idx);

                // Collect schemas under this db
                let mut all_schemas: std::collections::BTreeSet<String> =
                    std::collections::BTreeSet::new();
                if let Some(schemas) = db_schemas.get(db_name) {
                    for schema in schemas.keys() {
                        all_schemas.insert(schema.clone());
                    }
                }
                if let Some(schemas) = schema_only.get(db_name) {
                    for schema in schemas.keys() {
                        all_schemas.insert(schema.clone());
                    }
                }

                for schema_name in &all_schemas {
                    let schema_result_idx = schema_only
                        .get(db_name)
                        .and_then(|s| s.get(schema_name))
                        .and_then(|v| v.first())
                        .map(|(idx, _)| *idx);

                    let has_tables = db_schemas
                        .get(db_name)
                        .and_then(|s| s.get(schema_name))
                        .is_some_and(|v| !v.is_empty());

                    if has_tables {
                        lines.push(TreeLine {
                            indent: 1,
                            text: schema_name.clone(),
                            is_header: true,
                        });
                        line_to_result.push(schema_result_idx);

                        if let Some(tables) =
                            db_schemas.get(db_name).and_then(|s| s.get(schema_name))
                        {
                            for (result_idx, entry) in tables {
                                let name = entry.table.as_deref().unwrap_or("");
                                let suffix = if entry.is_view { " (V)" } else { "" };
                                lines.push(TreeLine {
                                    indent: 2,
                                    text: format!("{name}{suffix}"),
                                    is_header: false,
                                });
                                line_to_result.push(Some(*result_idx));
                            }
                        }
                    } else {
                        // Schema with no table children -- render as leaf
                        lines.push(TreeLine {
                            indent: 1,
                            text: schema_name.clone(),
                            is_header: false,
                        });
                        line_to_result.push(schema_result_idx);
                    }
                }
            } else if let Some(entries) = db_only.get(db_name) {
                // Database-only match with no children -- render as leaf
                for (result_idx, _) in entries {
                    lines.push(TreeLine {
                        indent: 0,
                        text: db_name.clone(),
                        is_header: false,
                    });
                    line_to_result.push(Some(*result_idx));
                }
            }
        }

        (lines, line_to_result)
    }
}

struct TreeLine {
    indent: usize,
    text: String,
    is_header: bool,
}

impl Component for FuzzySearch {
    fn update(&mut self, _action: Action) -> Result<Option<Action>> {
        Ok(None)
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        if !self.active {
            return Ok(None);
        }

        match key.code {
            KeyCode::Esc => {
                self.deactivate();
                Ok(Some(Action::Render))
            }
            KeyCode::Enter => {
                // Selection is handled by the parent (ExploreState)
                // Return a Render action; parent will check selected_entry()
                Ok(Some(Action::Render))
            }
            KeyCode::Down => {
                if !self.results.is_empty() {
                    self.selected = (self.selected + 1).min(self.results.len() - 1);
                }
                self.adjust_scroll();
                Ok(Some(Action::Render))
            }
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                self.adjust_scroll();
                Ok(Some(Action::Render))
            }
            KeyCode::Char(c) => {
                self.input.push(c);
                self.update_results();
                Ok(Some(Action::Render))
            }
            KeyCode::Backspace => {
                self.input.pop();
                self.update_results();
                Ok(Some(Action::Render))
            }
            _ => Ok(None),
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        if !self.active {
            return;
        }

        // Render as overlay in the given area
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::dialog_border()))
            .title(" Search (Esc to cancel) ")
            .title_style(Style::default().fg(theme::accent_dim()))
            .style(Style::default().bg(theme::dialog_bg()).fg(theme::fg()));

        // Clear the area first
        frame.render_widget(Clear, area);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.height < 2 || inner.width < 2 {
            return;
        }

        // Input line at bottom of overlay
        let input_area = Rect {
            x: inner.x,
            y: inner.y + inner.height.saturating_sub(1),
            width: inner.width,
            height: 1,
        };

        let results_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: inner.height.saturating_sub(1),
        };

        // When there's a filter active, show tree-structured results
        if self.input.is_empty() {
            // No filter: show flat list (all entries)
            let max_lines = results_area.height as usize;
            let mut items: Vec<ListItem> = Vec::new();

            for (result_idx, &idx) in self.results.iter().enumerate().skip(self.scroll_offset) {
                if items.len() >= max_lines {
                    break;
                }
                let entry = &self.entries[idx];
                let is_selected = result_idx == self.selected;
                let style = if is_selected {
                    Style::default()
                        .bg(theme::bg_highlight())
                        .fg(theme::fg_bright())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme::fg())
                };
                items.push(ListItem::new(Line::from(Span::styled(
                    entry.display.clone(),
                    style,
                ))));
            }

            let list = List::new(items).style(Style::default().bg(theme::dialog_bg()).fg(theme::fg()));
            frame.render_widget(list, results_area);
        } else {
            let (tree_lines, line_to_result) = self.build_tree_lines();

            // We need to figure out scroll based on the line that contains the selected result.
            // Find which line corresponds to the currently selected result.
            let selected_line_idx = line_to_result
                .iter()
                .position(|r| *r == Some(self.selected))
                .unwrap_or(0);

            let max_lines = results_area.height as usize;
            // Adjust scroll so selected line is visible
            let scroll = if selected_line_idx >= self.scroll_offset + max_lines {
                selected_line_idx.saturating_sub(max_lines - 1)
            } else if selected_line_idx < self.scroll_offset {
                selected_line_idx
            } else {
                self.scroll_offset
            };

            let mut items: Vec<ListItem> = Vec::new();
            for (line_idx, tree_line) in tree_lines.iter().enumerate().skip(scroll) {
                if items.len() >= max_lines {
                    break;
                }

                let indent_str = "  ".repeat(tree_line.indent);
                let is_selected = line_to_result
                    .get(line_idx)
                    .and_then(|r| *r)
                    .is_some_and(|r| r == self.selected);

                let style = if is_selected {
                    Style::default()
                        .bg(theme::bg_highlight())
                        .fg(theme::fg_bright())
                        .add_modifier(Modifier::BOLD)
                } else if tree_line.is_header {
                    Style::default().fg(theme::accent())
                } else {
                    Style::default().fg(theme::fg())
                };

                let prefix = if tree_line.is_header {
                    "\u{25bc} " // down triangle
                } else {
                    "  "
                };

                items.push(ListItem::new(Line::from(Span::styled(
                    format!("{}{}{}", indent_str, prefix, tree_line.text),
                    style,
                ))));
            }

            let list = List::new(items).style(Style::default().bg(theme::dialog_bg()).fg(theme::fg()));
            frame.render_widget(list, results_area);
        }

        // Render input line
        let input_text = format!("/ {}", self.input);
        let input_paragraph = Paragraph::new(input_text)
            .style(Style::default().fg(theme::fg_bright()).bg(theme::dialog_bg()));
        frame.render_widget(input_paragraph, input_area);
    }
}
