#![allow(clippy::too_many_lines)]

use color_eyre::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Style;
use ratatui::widgets::Block;

use crate::action::Action;
use crate::components::Component;
use crate::components::background_tasks::{BackgroundTaskManager, BackgroundTasksPane};
use crate::components::explore::ExploreState;
use crate::components::query::QueryPane;
use crate::components::quit_dialog::QuitDialog;
use crate::components::saved_queries::SavedQueriesPane;
use crate::components::settings::SettingsPane;
use crate::components::top_bar::TopBar;
use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Explore,
    Query,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    TopBar,
    ModeContent,
}

pub struct App {
    pub(crate) mode: Mode,
    pub(crate) focus: Focus,
    pub(crate) connected: Option<bool>, // None = checking, Some(true) = connected, Some(false) = disconnected
    pub(crate) running: bool,

    pub(crate) top_bar: TopBar,
    pub(crate) quit_dialog: QuitDialog,

    // Mode components
    pub(crate) explore: ExploreState,
    pub(crate) query: QueryPane,
    pub(crate) settings: SettingsPane,

    // Background tasks
    pub(crate) task_manager: BackgroundTaskManager,
    pub(crate) background_tasks_pane: BackgroundTasksPane,

    // Saved queries
    pub(crate) saved_queries_pane: SavedQueriesPane,
    /// The ID of the saved query currently loaded in the editor (if any)
    pub(crate) loaded_saved_query_id: Option<usize>,
}

impl App {
    pub fn new() -> Self {
        Self {
            mode: Mode::Explore,
            focus: Focus::TopBar,
            connected: None,
            running: true,

            top_bar: TopBar::new(),
            quit_dialog: QuitDialog::new(),

            explore: ExploreState::new(),
            query: QueryPane::new(),
            settings: SettingsPane::new(),

            task_manager: BackgroundTaskManager::new(),
            background_tasks_pane: BackgroundTasksPane::new(),
            saved_queries_pane: SavedQueriesPane::new(),
            loaded_saved_query_id: None,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        // Quit dialog captures all events when visible
        if self.quit_dialog.visible {
            return self.quit_dialog.handle_key(key);
        }

        // Background tasks pane captures all events when visible
        if self.background_tasks_pane.visible {
            return self.background_tasks_pane.handle_key(key);
        }

        // Saved queries pane captures all events when visible
        if self.saved_queries_pane.visible {
            return self.saved_queries_pane.handle_key(key);
        }

        // Ctrl-D shows quit dialog
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('d') {
            return Ok(Some(Action::ShowQuitDialog));
        }

        // Ctrl+B: toggle background tasks
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('b') {
            return Ok(Some(Action::ToggleBackgroundTasks));
        }

        // Ctrl+L: toggle saved queries (Load) — Query mode only
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && key.code == KeyCode::Char('l')
            && self.mode == Mode::Query
        {
            return Ok(Some(Action::ToggleSavedQueries));
        }

        // Tab / Shift+Tab: try component first, then cycle between panes
        if key.code == KeyCode::Tab && !key.modifiers.contains(KeyModifiers::SHIFT) {
            match self.focus {
                Focus::TopBar => {
                    self.focus = Focus::ModeContent;
                    self.top_bar.focused = false;
                    self.set_first_subpane();
                }
                Focus::ModeContent => {
                    // Let the component try to handle Tab first (e.g., editor inserts tab)
                    let component = self.active_component_mut();
                    if let Some(action) = component.handle_key(key)? {
                        return Ok(Some(action));
                    }
                    // Component didn't consume it -- do pane cycling
                    if !self.advance_subpane() {
                        self.focus = Focus::TopBar;
                        self.top_bar.focused = true;
                    }
                }
            }
            return Ok(Some(Action::Render));
        }
        if key.code == KeyCode::BackTab {
            match self.focus {
                Focus::TopBar => {
                    self.focus = Focus::ModeContent;
                    self.top_bar.focused = false;
                    self.set_last_subpane();
                }
                Focus::ModeContent => {
                    if !self.retreat_subpane() {
                        self.focus = Focus::TopBar;
                        self.top_bar.focused = true;
                    }
                }
            }
            return Ok(Some(Action::Render));
        }

        // Escape: first let mode component handle it (close popups/overlays),
        // then move focus to TopBar, then from TopBar back to ModeContent
        if key.code == KeyCode::Esc {
            match self.focus {
                Focus::ModeContent => {
                    // Let mode component try to handle Esc first (close overlays)
                    let component = self.active_component_mut();
                    if let Some(action) = component.handle_key(key)? {
                        return Ok(Some(action));
                    }
                    // Not consumed by component — move focus to TopBar
                    self.focus = Focus::TopBar;
                    self.top_bar.focused = true;
                    return Ok(Some(Action::Render));
                }
                Focus::TopBar => {
                    self.focus = Focus::ModeContent;
                    self.top_bar.focused = false;
                    return Ok(Some(Action::Render));
                }
            }
        }

        // Enter from TopBar returns to ModeContent
        if self.focus == Focus::TopBar && key.code == KeyCode::Enter {
            self.focus = Focus::ModeContent;
            self.top_bar.focused = false;
            return Ok(Some(Action::Render));
        }

        // Route to focused component
        match self.focus {
            Focus::TopBar => self.top_bar.handle_key(key),
            Focus::ModeContent => {
                let component = self.active_component_mut();
                component.handle_key(key)
            }
        }
    }

    pub fn dispatch(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::Quit => {
                self.running = false;
                Ok(None)
            }
            Action::ShowQuitDialog => {
                self.quit_dialog.show();
                Ok(Some(Action::Render))
            }
            Action::HideQuitDialog => {
                self.quit_dialog.visible = false;
                Ok(Some(Action::Render))
            }
            Action::SwitchMode(mode) => {
                // Reset nvim spawn counter when switching modes so the user can
                // retry after a crash loop by switching away and back to Query.
                self.query.editor_mut().reset_spawn_counter();
                self.mode = mode;
                self.focus = Focus::ModeContent;
                self.top_bar.focused = false;
                self.set_first_subpane();
                Ok(Some(Action::Render))
            }
            Action::HeartbeatResult(ok) => {
                self.connected = Some(ok);
                Ok(Some(Action::Render))
            }
            Action::ToggleBackgroundTasks => {
                if !self.background_tasks_pane.visible {
                    // Close other overlays first
                    self.saved_queries_pane.visible = false;
                }
                self.background_tasks_pane.toggle();
                Ok(Some(Action::Render))
            }
            Action::ToggleSavedQueries => {
                if !self.saved_queries_pane.visible {
                    // Close other overlays first
                    self.background_tasks_pane.visible = false;
                }
                self.saved_queries_pane.toggle();
                Ok(Some(Action::Render))
            }
            Action::CreateTask {
                ref source,
                ref name,
            } => {
                let id = self.task_manager.create_task(source, name);
                self.task_manager.start_task(id);
                // Update refresh counts if it's a catalog refresh task
                if source == "Catalog Refresh" {
                    let running = self.task_manager.running_count("Catalog Refresh");
                    let total = self
                        .task_manager
                        .pending_or_running_count("Catalog Refresh");
                    self.explore.tree.refresh_running = running;
                    self.explore.tree.refresh_total = total;
                    self.settings.refresh_running = running;
                    self.settings.refresh_total = total;
                }
                Ok(Some(Action::Render))
            }
            Action::CompleteTask(id) => {
                self.task_manager.complete_task(id);
                // Update refresh counts on tree and settings
                let running = self.task_manager.running_count("Catalog Refresh");
                let total = self
                    .task_manager
                    .pending_or_running_count("Catalog Refresh");
                self.explore.tree.refresh_running = running;
                self.explore.tree.refresh_total = total;
                self.settings.refresh_running = running;
                self.settings.refresh_total = total;
                Ok(Some(Action::Render))
            }
            Action::LoadSavedQuery { id, ref sql } => {
                self.loaded_saved_query_id = Some(id);
                let sql_clone = sql.clone();
                // Restore chart configs from the saved query
                if let Some(saved) = self.saved_queries_pane.queries.iter().find(|q| q.id == id) {
                    self.query.restore_chart_configs(saved.charts.clone());
                }
                self.query.update(Action::LoadQueryFromHistory(sql_clone))?;
                Ok(Some(Action::Render))
            }
            Action::SaveQueryToHistory => {
                // Handled by main.rs
                Ok(Some(action))
            }
            Action::PersistSavedQueries => {
                // Clear loaded_saved_query_id if the saved query was deleted
                if let Some(id) = self.loaded_saved_query_id
                    && !self.saved_queries_pane.queries.iter().any(|q| q.id == id)
                {
                    self.loaded_saved_query_id = None;
                }
                // Handled by main.rs
                Ok(Some(action))
            }
            Action::NewQuery => {
                self.loaded_saved_query_id = None;
                self.query.update(Action::NewQuery)?;
                Ok(Some(Action::Render))
            }
            // These actions need to be handled by main.rs (async/terminal access)
            Action::ExecuteQuery(_)
            | Action::ApplyOverrides { .. }
            | Action::ClearCache
            | Action::SaveTheme(_)
            | Action::ResetTheme
            // Async RPC operations — also forwarded to main.rs
            | Action::SetEditorContent(_)
            | Action::PushCatalogToNvim(_) => Ok(Some(action)),
            // Nvim lifecycle events always go to query pane regardless of active mode
            Action::NvimReady | Action::NvimExited(_) | Action::NvimRpcConnected => {
                self.query.update(action)
            }
            // Theme saved — route to settings
            Action::ThemeSaved => self.settings.update(Action::ThemeSaved),
            Action::Render | Action::None => Ok(None),
            _ => {
                // Forward to active mode component
                let component = self.active_component_mut();
                component.update(action)
            }
        }
    }

    fn active_component_mut(&mut self) -> &mut dyn Component {
        match self.mode {
            Mode::Explore => &mut self.explore,
            Mode::Query => &mut self.query,
            Mode::Settings => &mut self.settings,
        }
    }

    /// Set focus to the first sub-pane of the current mode
    fn set_first_subpane(&mut self) {
        match self.mode {
            Mode::Explore => {
                self.explore.app_focused = true;
                self.explore.focus = crate::components::explore::ExploreFocus::Tree;
            }
            Mode::Query => {
                self.query.app_focused = true;
                self.query.set_focus_editor();
            }
            Mode::Settings => {
                self.settings.app_focused = true;
                self.settings.focus = crate::components::settings::SettingsFocus::Connection;
            }
        }
    }

    /// Set focus to the last sub-pane of the current mode
    fn set_last_subpane(&mut self) {
        match self.mode {
            Mode::Explore => {
                self.explore.app_focused = true;
                self.explore.focus = crate::components::explore::ExploreFocus::Detail;
            }
            Mode::Query => {
                self.query.app_focused = true;
                self.query.set_focus_results();
            }
            Mode::Settings => {
                self.settings.app_focused = true;
                self.settings.focus = crate::components::settings::SettingsFocus::Theme;
            }
        }
    }

    /// Advance to the next sub-pane. Returns true if advanced, false if at the last sub-pane.
    fn advance_subpane(&mut self) -> bool {
        match self.mode {
            Mode::Explore => match self.explore.focus {
                crate::components::explore::ExploreFocus::Tree => {
                    self.explore.focus = crate::components::explore::ExploreFocus::Detail;
                    true
                }
                crate::components::explore::ExploreFocus::Detail => false,
            },
            Mode::Query => self.query.advance_focus(),
            Mode::Settings => match self.settings.focus {
                crate::components::settings::SettingsFocus::Connection => {
                    self.settings.focus = crate::components::settings::SettingsFocus::Cache;
                    true
                }
                crate::components::settings::SettingsFocus::Cache => {
                    self.settings.focus = crate::components::settings::SettingsFocus::Theme;
                    true
                }
                crate::components::settings::SettingsFocus::Theme => false,
            },
        }
    }

    /// Retreat to the previous sub-pane. Returns true if retreated, false if at the first sub-pane.
    fn retreat_subpane(&mut self) -> bool {
        match self.mode {
            Mode::Explore => match self.explore.focus {
                crate::components::explore::ExploreFocus::Detail => {
                    self.explore.focus = crate::components::explore::ExploreFocus::Tree;
                    true
                }
                crate::components::explore::ExploreFocus::Tree => false,
            },
            Mode::Query => self.query.retreat_focus(),
            Mode::Settings => match self.settings.focus {
                crate::components::settings::SettingsFocus::Theme => {
                    self.settings.focus = crate::components::settings::SettingsFocus::Cache;
                    true
                }
                crate::components::settings::SettingsFocus::Cache => {
                    self.settings.focus = crate::components::settings::SettingsFocus::Connection;
                    true
                }
                crate::components::settings::SettingsFocus::Connection => false,
            },
        }
    }

    pub fn render(&mut self, frame: &mut Frame) {
        let size = frame.area();

        // Minimum terminal size check
        if size.width < 40 || size.height < 10 {
            use ratatui::layout::Alignment;
            use ratatui::widgets::Paragraph;
            let msg = Paragraph::new("Terminal too small\nMinimum: 40x10")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme::fg()).bg(theme::bg()));
            frame.render_widget(msg, size);
            return;
        }

        // Fill background
        let bg = Block::default().style(Style::default().bg(theme::bg()).fg(theme::fg()));
        frame.render_widget(bg, size);

        // Layout: top bar always 3 lines (border always shown) + content
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(size);

        // Render top bar with state
        self.top_bar
            .render_with_state(frame, chunks[0], self.mode, self.connected);

        // Render active mode content
        let mode_focused = self.focus == Focus::ModeContent;
        match self.mode {
            Mode::Explore => {
                self.explore.app_focused = mode_focused;
                self.explore.render(frame, chunks[1]);
            }
            Mode::Query => {
                self.query.app_focused = mode_focused;
                self.query.render(frame, chunks[1]);
            }
            Mode::Settings => {
                self.settings.app_focused = mode_focused;
                self.settings.render(frame, chunks[1]);
            }
        }

        // Render background tasks overlay
        self.background_tasks_pane
            .render(frame, size, &self.task_manager);

        // Render saved queries overlay
        self.saved_queries_pane.render(frame, size);

        // Render quit dialog overlay (on top of everything)
        self.quit_dialog.render(frame, size);
    }
}
