use std::cell::RefCell;
use std::collections::HashSet;

use color_eyre::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use tui_tree_widget::{Tree, TreeItem, TreeState};

use crate::action::Action;
use crate::components::Component;
use crate::snowflake::{Database, Schema, Table};
use crate::theme;

/// Catalog tree showing Database -> Schema -> Table/View hierarchy
pub struct CatalogTree {
    pub databases: Vec<Database>,
    pub schemas: Vec<Schema>,
    pub tables: Vec<Table>,
    pub tree_state: RefCell<TreeState<String>>,
    pub last_refreshed: Option<chrono::DateTime<chrono::Utc>>,
    pub loading: bool,
    pub error: Option<String>,
    pub refresh_running: usize,
    pub refresh_total: usize,
    /// Groups that have already been auto-opened (to avoid re-opening on every render)
    auto_opened: RefCell<HashSet<String>>,
}

impl CatalogTree {
    pub fn new() -> Self {
        Self {
            databases: Vec::new(),
            schemas: Vec::new(),
            tables: Vec::new(),
            tree_state: RefCell::new(TreeState::default()),
            last_refreshed: None,
            loading: false,
            error: None,
            refresh_running: 0,
            refresh_total: 0,
            auto_opened: RefCell::new(HashSet::new()),
        }
    }

    pub fn clear(&mut self) {
        self.databases.clear();
        self.schemas.clear();
        self.tables.clear();
        self.tree_state = RefCell::new(TreeState::default());
        self.last_refreshed = None;
        self.loading = false;
        self.error = None;
        self.refresh_running = 0;
        self.refresh_total = 0;
        self.auto_opened.borrow_mut().clear();
    }

    pub fn set_catalog(
        &mut self,
        databases: std::sync::Arc<Vec<Database>>,
        schemas: std::sync::Arc<Vec<Schema>>,
        tables: std::sync::Arc<Vec<Table>>,
        updated_at: Option<chrono::DateTime<chrono::Utc>>,
    ) {
        self.databases = std::sync::Arc::try_unwrap(databases).unwrap_or_else(|a| (*a).clone());
        self.schemas = std::sync::Arc::try_unwrap(schemas).unwrap_or_else(|a| (*a).clone());
        self.tables = std::sync::Arc::try_unwrap(tables).unwrap_or_else(|a| (*a).clone());
        self.last_refreshed = Some(updated_at.unwrap_or_else(chrono::Utc::now));
        self.loading = false;
        self.error = None;
        self.auto_opened.borrow_mut().clear();
    }

    pub fn set_loading(&mut self) {
        self.loading = true;
        self.error = None;
    }

    pub fn set_error(&mut self, msg: String) {
        self.error = Some(msg);
        self.loading = false;
    }

    /// Build `TreeItem` list from catalog data, grouping Tables and Views under each schema
    fn build_tree_items(&self) -> Vec<TreeItem<'_, String>> {
        // Pre-partition tables by (db, schema) and kind to avoid O(schemas * tables) filtering
        let mut tables_by_schema: std::collections::HashMap<(&str, &str), Vec<&Table>> =
            std::collections::HashMap::new();
        let mut views_by_schema: std::collections::HashMap<(&str, &str), Vec<&Table>> =
            std::collections::HashMap::new();
        for t in &self.tables {
            let key = (t.database.as_str(), t.schema.as_str());
            if t.kind.eq_ignore_ascii_case("VIEW") {
                views_by_schema.entry(key).or_default().push(t);
            } else {
                tables_by_schema.entry(key).or_default().push(t);
            }
        }

        let mut db_items: Vec<TreeItem<String>> = Vec::new();

        for db in &self.databases {
            let db_schemas: Vec<&Schema> = self
                .schemas
                .iter()
                .filter(|s| s.database == db.name)
                .collect();

            let mut schema_items: Vec<TreeItem<String>> = Vec::new();

            for schema in &db_schemas {
                let key = (db.name.as_str(), schema.name.as_str());
                let empty_tables = vec![];
                let empty_views = vec![];
                let schema_tables = tables_by_schema.get(&key).unwrap_or(&empty_tables);
                let schema_views = views_by_schema.get(&key).unwrap_or(&empty_views);

                let has_tables = !schema_tables.is_empty();
                let has_views = !schema_views.is_empty();
                let has_only_one_type = has_tables != has_views; // XOR: only one type present

                let mut group_items: Vec<TreeItem<String>> = Vec::new();

                if has_tables {
                    let table_leaves: Vec<TreeItem<String>> = schema_tables
                        .iter()
                        .map(|t| {
                            let id = format!("{}.{}.{}", db.name, schema.name, t.name);
                            TreeItem::new_leaf(id, t.name.clone())
                        })
                        .collect();

                    let group_id = format!("{}.{}.~tables", db.name, schema.name);
                    let label = Line::from(Span::styled(
                        "Tables",
                        Style::default()
                            .fg(theme::fg_dim())
                            .add_modifier(Modifier::ITALIC),
                    ));
                    if let Ok(item) = TreeItem::new(group_id.clone(), label, table_leaves) {
                        group_items.push(item);
                        // Auto-unfold once if only one type present
                        if has_only_one_type
                            && self.auto_opened.borrow_mut().insert(group_id.clone())
                        {
                            let schema_id = format!("{}.{}", db.name, schema.name);
                            self.tree_state.borrow_mut().open(vec![
                                db.name.clone(),
                                schema_id,
                                group_id,
                            ]);
                        }
                    }
                }

                if has_views {
                    let view_leaves: Vec<TreeItem<String>> = schema_views
                        .iter()
                        .map(|t| {
                            let id = format!("{}.{}.{}", db.name, schema.name, t.name);
                            TreeItem::new_leaf(id, t.name.clone())
                        })
                        .collect();

                    let group_id = format!("{}.{}.~views", db.name, schema.name);
                    let label = Line::from(Span::styled(
                        "Views",
                        Style::default()
                            .fg(theme::fg_dim())
                            .add_modifier(Modifier::ITALIC),
                    ));
                    if let Ok(item) = TreeItem::new(group_id.clone(), label, view_leaves) {
                        group_items.push(item);
                        // Auto-unfold once if only one type present
                        if has_only_one_type
                            && self.auto_opened.borrow_mut().insert(group_id.clone())
                        {
                            let schema_id = format!("{}.{}", db.name, schema.name);
                            self.tree_state.borrow_mut().open(vec![
                                db.name.clone(),
                                schema_id,
                                group_id,
                            ]);
                        }
                    }
                }

                let schema_id = format!("{}.{}", db.name, schema.name);
                if let Ok(item) = TreeItem::new(schema_id, schema.name.clone(), group_items) {
                    schema_items.push(item);
                }
            }

            if let Ok(item) = TreeItem::new(db.name.clone(), db.name.clone(), schema_items) {
                db_items.push(item);
            }
        }

        db_items
    }

    /// Get the Table metadata for a given db.schema.table
    pub fn find_table(&self, db: &str, schema: &str, table: &str) -> Option<Table> {
        self.tables
            .iter()
            .find(|t| t.database == db && t.schema == schema && t.name == table)
            .cloned()
    }

    /// Get the currently selected table/view info (db, schema, table) if a leaf node is selected
    pub fn selected_table(&self) -> Option<(String, String, String)> {
        let state = self.tree_state.borrow();
        let selected = state.selected();
        if selected.is_empty() {
            return None;
        }

        // With groupings, a table/view is at depth 4:
        // [db_id, schema_id, group_id (~tables/~views), table_id (db.schema.table)]
        if selected.len() == 4 {
            let table_id = &selected[3]; // "db.schema.table"
            let parts: Vec<&str> = table_id.splitn(3, '.').collect();
            if parts.len() == 3 {
                return Some((
                    parts[0].to_string(),
                    parts[1].to_string(),
                    parts[2].to_string(),
                ));
            }
        }

        None
    }

    fn refresh_status(&self) -> String {
        match self.last_refreshed {
            Some(ts) => format!("Refreshed: {}", format_relative_time(ts)),
            None => "Not yet loaded".to_string(),
        }
    }
}

impl Component for CatalogTree {
    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::CatalogLoaded(databases, schemas, tables, updated_at) => {
                self.set_catalog(databases, schemas, tables, updated_at);
                Ok(Some(Action::Render))
            }
            Action::DatabasesLoaded(databases) => {
                self.databases = databases;
                self.loading = false;
                self.error = None;
                Ok(Some(Action::Render))
            }
            Action::ObjectsLoaded(schemas, tables) => {
                self.schemas = schemas;
                self.tables = tables;
                self.last_refreshed = Some(chrono::Utc::now());
                Ok(Some(Action::Render))
            }
            Action::CachedColumnsRefreshed => Ok(Some(Action::Render)),
            Action::CatalogError(msg) => {
                self.set_error(msg);
                Ok(Some(Action::Render))
            }
            Action::DatabaseCatalogLoaded {
                ref db,
                schemas,
                tables,
            } => {
                // Remove old entries for this database before adding new ones
                self.schemas.retain(|s| s.database != *db);
                self.tables.retain(|t| t.database != *db);
                self.schemas
                    .extend(std::sync::Arc::try_unwrap(schemas).unwrap_or_else(|a| (*a).clone()));
                self.tables
                    .extend(std::sync::Arc::try_unwrap(tables).unwrap_or_else(|a| (*a).clone()));
                self.last_refreshed = Some(chrono::Utc::now());
                Ok(Some(Action::Render))
            }
            _ => Ok(None),
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                self.tree_state.borrow_mut().key_down();
                if let Some((db, schema, table)) = self.selected_table() {
                    return Ok(Some(Action::LoadColumns { db, schema, table }));
                }
                Ok(Some(Action::Render))
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.tree_state.borrow_mut().key_up();
                if let Some((db, schema, table)) = self.selected_table() {
                    return Ok(Some(Action::LoadColumns { db, schema, table }));
                }
                Ok(Some(Action::Render))
            }
            KeyCode::Right => {
                self.tree_state.borrow_mut().key_right();
                if let Some((db, schema, table)) = self.selected_table() {
                    return Ok(Some(Action::LoadColumns { db, schema, table }));
                }
                Ok(Some(Action::Render))
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.tree_state.borrow_mut().toggle_selected();
                if let Some((db, schema, table)) = self.selected_table() {
                    return Ok(Some(Action::LoadColumns { db, schema, table }));
                }
                Ok(Some(Action::Render))
            }
            KeyCode::Left => {
                self.tree_state.borrow_mut().key_left();
                Ok(Some(Action::Render))
            }
            KeyCode::Char('R') => Ok(Some(Action::RefreshCatalog)),
            _ => Ok(None),
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        self.render_with_focus(frame, area, false);
    }
}

impl CatalogTree {
    pub fn render_with_focus(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let border_color = if focused {
            theme::border_focus()
        } else {
            theme::border()
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(" Catalog ")
            .title_style(Style::default().fg(theme::accent_dim()))
            .style(Style::default().bg(theme::bg_surface()).fg(theme::fg()));

        let raw_inner = block.inner(area);
        frame.render_widget(block, area);

        if raw_inner.height < 3 || raw_inner.width < 4 {
            return;
        }

        // Add 1-char left padding
        let inner = Rect {
            x: raw_inner.x + 1,
            width: raw_inner.width.saturating_sub(1),
            ..raw_inner
        };

        // Split: top padding + tree content + hint bar + status bar at bottom
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // top padding
                Constraint::Min(0),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);

        // Render the tree or status messages
        if self.loading {
            let paragraph =
                Paragraph::new("Loading catalog...").style(Style::default().fg(theme::fg_dim()));
            frame.render_widget(paragraph, chunks[1]);
        } else if let Some(ref err) = self.error {
            let paragraph =
                Paragraph::new(format!("Error: {err}")).style(Style::default().fg(theme::red()));
            frame.render_widget(paragraph, chunks[1]);
        } else if self.databases.is_empty() {
            let paragraph = Paragraph::new("No databases loaded. Press R to refresh.")
                .style(Style::default().fg(theme::fg_dim()));
            frame.render_widget(paragraph, chunks[1]);
        } else {
            let items = self.build_tree_items();
            if let Ok(tree) = Tree::new(&items) {
                let highlight = if focused {
                    Style::default()
                        .bg(theme::bg_highlight())
                        .fg(theme::fg_bright())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().bg(theme::bg_highlight()).fg(theme::fg_dim())
                };
                let tree = tree
                    .highlight_style(highlight)
                    .node_closed_symbol("\u{25b6} ")
                    .node_open_symbol("\u{25bc} ")
                    .node_no_children_symbol("  ");

                let mut state = self.tree_state.borrow_mut();
                frame.render_stateful_widget(tree, chunks[1], &mut state);
            }
        }

        // Hint bar — only show pane-specific shortcuts when focused
        let hint_line = if focused {
            let accent = Style::default().fg(theme::accent_dim());
            let dim = Style::default().fg(theme::fg_dim());
            Line::from(vec![
                Span::styled(" /", accent),
                Span::styled(": Search  ", dim),
                Span::styled("R", accent),
                Span::styled(": Refresh  ", dim),
                Span::styled("Tab", accent),
                Span::styled(": Detail", dim),
            ])
        } else {
            Line::from("")
        };
        let hint_paragraph =
            Paragraph::new(hint_line).style(Style::default().bg(theme::bg_surface()).fg(theme::fg()));
        frame.render_widget(hint_paragraph, chunks[2]);

        // Status bar at bottom — show refresh progress using task counts, or last-refresh time
        let status_line = if self.refresh_total > 0 {
            Line::from(Span::styled(
                format!(
                    "Running refresh tasks: {}/{}",
                    self.refresh_running, self.refresh_total
                ),
                Style::default().fg(theme::yellow()),
            ))
        } else {
            let status = self.refresh_status();
            Line::from(Span::styled(status, Style::default().fg(theme::fg_dim())))
        };
        let status_paragraph =
            Paragraph::new(status_line).style(Style::default().bg(theme::bg_surface()).fg(theme::fg()));
        frame.render_widget(status_paragraph, chunks[3]);
    }
}

fn plural(n: i64, unit: &str) -> String {
    if n == 1 {
        format!("{n} {unit}")
    } else {
        format!("{n} {unit}s")
    }
}

fn format_relative_time(ts: chrono::DateTime<chrono::Utc>) -> String {
    let ago = chrono::Utc::now().signed_duration_since(ts);
    let mins = ago.num_minutes();
    let hours = ago.num_hours();
    let days = ago.num_days();
    let weeks = days / 7;
    let months = days / 30;
    let years = days / 365;

    if mins < 1 {
        "just now".into()
    } else if hours < 1 {
        format!("{} ago", plural(mins, "min"))
    } else if days < 1 {
        let m = mins % 60;
        format!("{hours}h {m}m ago")
    } else if months < 1 {
        format!("{} ago", plural(days, "day"))
    } else if months < 3 {
        format!("{} ago", plural(weeks, "week"))
    } else if years < 1 {
        format!("{} ago", plural(months, "month"))
    } else {
        let rem = months % 12;
        format!("{years}y {rem}mo ago")
    }
}
