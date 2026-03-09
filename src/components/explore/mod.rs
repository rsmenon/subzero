pub mod columns_tab;
pub mod detail;
pub mod preview_tab;
pub mod search;
pub mod tree;

use color_eyre::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::action::Action;
use crate::components::Component;

use self::detail::DetailPanel;
use self::search::FuzzySearch;
use self::tree::CatalogTree;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExploreFocus {
    Tree,
    Detail,
}

pub struct ExploreState {
    pub focus: ExploreFocus,
    pub app_focused: bool,
    pub tree: CatalogTree,
    pub detail: DetailPanel,
    pub search: FuzzySearch,
    pub catalog_loaded: bool,
}

impl ExploreState {
    pub fn new() -> Self {
        Self {
            focus: ExploreFocus::Tree,
            app_focused: true,
            tree: CatalogTree::new(),
            detail: DetailPanel::new(),
            search: FuzzySearch::new(),
            catalog_loaded: false,
        }
    }

    /// Check if a db/schema/table triple matches the currently selected table in the tree.
    /// Used as a staleness guard for async responses.
    fn matches_selected_table(&self, db: &str, schema: &str, table: &str) -> bool {
        self.tree
            .selected_table()
            .is_some_and(|(cur_db, cur_schema, cur_table)| {
                cur_db == db && cur_schema == schema && cur_table == table
            })
    }

    /// Navigate the tree to a search result
    fn navigate_to_search_result(&mut self) -> Option<Action> {
        let entry = self.search.selected_entry()?.clone();
        self.search.deactivate();

        let mut state = self.tree.tree_state.borrow_mut();

        // Open the database node
        state.open(vec![entry.db.clone()]);

        if entry.schema.is_empty() {
            state.select(vec![entry.db.clone()]);
        } else {
            // Open the schema node
            let schema_id = format!("{}.{}", entry.db, entry.schema);
            state.open(vec![entry.db.clone(), schema_id.clone()]);

            if let Some(ref table) = entry.table {
                // Determine if the item is a view or table by checking kind
                let is_view = entry.is_view;
                let group_suffix = if is_view { "~views" } else { "~tables" };
                let group_id = format!("{}.{}.{}", entry.db, entry.schema, group_suffix);

                // Open the group node
                state.open(vec![entry.db.clone(), schema_id.clone(), group_id.clone()]);

                // Select the table/view node
                let table_id = format!("{}.{}.{}", entry.db, entry.schema, table);
                state.select(vec![entry.db.clone(), schema_id, group_id, table_id]);
                drop(state);

                self.detail.set_breadcrumb(&entry.db, &entry.schema, table);
                self.detail
                    .set_table_meta(self.tree.find_table(&entry.db, &entry.schema, table));

                return Some(Action::LoadColumns {
                    db: entry.db,
                    schema: entry.schema,
                    table: table.clone(),
                });
            }
            state.select(vec![entry.db.clone(), schema_id]);
        }

        Some(Action::Render)
    }
}

impl Component for ExploreState {
    #[allow(clippy::match_same_arms)]
    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match &action {
            Action::CatalogLoaded(dbs, schemas, tables, _) => {
                self.catalog_loaded = true;
                self.search.rebuild_entries(dbs, schemas, tables);
                self.tree.update(action)
            }
            Action::DatabasesLoaded(_) => {
                self.catalog_loaded = true;
                self.tree.update(action)
            }
            Action::ObjectsLoaded(schemas, tables) => {
                // Rebuild search entries with the full data
                self.search
                    .rebuild_entries(&self.tree.databases, schemas, tables);
                self.tree.update(action)
            }
            Action::CachedColumnsRefreshed | Action::CatalogError(_) => {
                self.tree.update(action)
            }
            Action::ColumnsLoaded {
                db, schema, table, ..
            } => {
                if !self.matches_selected_table(db, schema, table) {
                    return Ok(None);
                }
                self.detail.set_breadcrumb(db, schema, table);
                self.detail
                    .set_table_meta(self.tree.find_table(db, schema, table));
                self.detail.active_tab = detail::DetailTab::Columns;
                self.detail.update(action)
            }
            Action::PreviewLoaded {
                db, schema, table, ..
            } => {
                if !self.matches_selected_table(db, schema, table) {
                    return Ok(None);
                }
                self.detail.update(action)
            }
            Action::ColumnsError {
                db, schema, table, ..
            } => {
                if !self.matches_selected_table(db, schema, table) {
                    return Ok(None);
                }
                self.detail.update(action)
            }
            Action::PreviewError {
                db, schema, table, ..
            } => {
                if !self.matches_selected_table(db, schema, table) {
                    return Ok(None);
                }
                self.detail.update(action)
            }
            _ => Ok(None),
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        // Search overlay captures all input when active
        if self.search.active {
            if key.code == KeyCode::Enter {
                // Handle selection
                if let Some(action) = self.navigate_to_search_result() {
                    return Ok(Some(action));
                }
                return Ok(Some(Action::Render));
            }
            return self.search.handle_key(key);
        }

        // Esc: close popups/overlays in detail pane first
        if key.code == KeyCode::Esc {
            if self.focus == ExploreFocus::Detail {
                // Row detail popup takes priority
                if self.detail.preview_tab.row_detail.visible {
                    self.detail.preview_tab.row_detail.handle_key(key);
                    return Ok(Some(Action::Render));
                }
                // Column search
                if self.detail.columns_tab.search_active() {
                    self.detail.columns_tab.handle_key(key)?;
                    return Ok(Some(Action::Render));
                }
            }
            // Otherwise don't consume — let app.rs handle pane navigation
            return Ok(None);
        }


        // '/' activates search when in tree
        if self.focus == ExploreFocus::Tree && key.code == KeyCode::Char('/') {
            self.search.activate();
            return Ok(Some(Action::Render));
        }

        // Route to focused component
        match self.focus {
            ExploreFocus::Tree => self.tree.handle_key(key),
            ExploreFocus::Detail => self.detail.handle_key(key),
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Split: 30% tree, 70% detail
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
            .split(area);

        let tree_focused = self.app_focused && self.focus == ExploreFocus::Tree;
        let detail_focused = self.app_focused && self.focus == ExploreFocus::Detail;
        self.tree.render_with_focus(frame, chunks[0], tree_focused);
        self.detail
            .render_with_focus(frame, chunks[1], detail_focused);

        // Render search overlay on top of the tree pane if active
        if self.search.active {
            // Use the tree area for the search overlay
            let search_area = chunks[0];
            self.search.render(frame, search_area);
        }
    }
}
