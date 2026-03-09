#![allow(clippy::cast_possible_truncation, clippy::too_many_lines)]

pub mod chart;
pub mod history;
pub mod nvim_editor;
pub mod nvim_rpc;
pub mod results;

use std::io::Write;
use std::path::PathBuf;

use color_eyre::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use tokio::sync::mpsc;

use crate::action::Action;
use crate::cache::store::QueryHistoryEntry;
use crate::components::Component;
use crate::event::AppEvent;

use self::chart::ChartPane;
use self::chart::config::ChartTab;
use self::history::QueryHistory;
use self::nvim_editor::NvimEditor;
use self::results::ResultsTable;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueryFocus {
    Editor,
    Results,
}

/// Which bottom tab is active: Results (table), or a specific chart tab index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BottomTab {
    Results,
    Chart(usize), // index into chart_tabs
}

pub struct QueryPane {
    editor: NvimEditor,
    results: ResultsTable,
    chart: ChartPane,
    history: QueryHistory,
    focus: QueryFocus,
    pub app_focused: bool,
    bottom_tab: BottomTab,
    split_pct: u16, // percentage of area for editor (top)
    status_message: Option<(String, std::time::Instant)>,
    query_history_entries: Vec<QueryHistoryEntry>,
    /// Whether nvim is available on PATH
    nvim_available: bool,
    /// Error message if nvim is not available
    nvim_error: Option<String>,
    /// Chart tab configs (managed here, not in `ChartPane`)
    chart_tabs: Vec<ChartTab>,
    /// Whether the delete chart confirmation popup is showing
    delete_confirm: bool,
    /// Title edit state: `Some((chart_index`, buffer, `original_label`))
    title_edit: Option<(usize, String, String)>,
}

impl QueryPane {
    pub fn new() -> Self {
        // Check for nvim availability at construction
        let (nvim_available, nvim_error) = match nvim_editor::check_nvim_available() {
            Ok(()) => (true, None),
            Err(e) => (false, Some(e)),
        };

        Self {
            editor: NvimEditor::new(),
            results: ResultsTable::new(),
            chart: ChartPane::new(),
            history: QueryHistory::new(),
            focus: QueryFocus::Editor,
            app_focused: true,
            bottom_tab: BottomTab::Results,
            split_pct: 40,
            status_message: None,
            query_history_entries: Vec::new(),
            nvim_available,
            nvim_error,
            chart_tabs: Vec::new(),
            delete_confirm: false,
            title_edit: None,
        }
    }

    /// Set the action tx on the nvim editor for lifecycle events
    pub fn set_action_tx(&mut self, tx: mpsc::UnboundedSender<AppEvent>) {
        self.editor.set_action_tx(tx);
    }

    pub fn set_history(&mut self, entries: Vec<QueryHistoryEntry>) {
        self.query_history_entries = entries;
    }

    pub fn get_sql(&mut self) -> String {
        self.editor.get_sql()
    }

    /// Get a reference to the nvim editor for RPC operations.
    pub fn editor_mut(&mut self) -> &mut NvimEditor {
        &mut self.editor
    }

    /// Get chart configs for saving alongside a saved query
    pub fn get_chart_configs(&self) -> Vec<ChartTab> {
        self.chart_tabs.clone()
    }

    /// Restore chart configs from a saved query
    pub fn restore_chart_configs(&mut self, tabs: Vec<ChartTab>) {
        self.chart_tabs = tabs;
        // Always start with settings closed
        for tab in &mut self.chart_tabs {
            tab.settings_hidden = true;
        }
        // If on a chart tab that no longer exists, go back to Results
        if let BottomTab::Chart(i) = self.bottom_tab
            && i >= self.chart_tabs.len()
        {
            self.bottom_tab = BottomTab::Results;
        }
    }

    fn update_focus_state(&mut self) {
        self.editor.focused = self.app_focused && self.focus == QueryFocus::Editor;
        self.results.focused = self.app_focused && self.focus == QueryFocus::Results;
    }

    fn show_status(&mut self, msg: String) {
        self.status_message = Some((msg, std::time::Instant::now()));
    }

    fn export_csv(&self) -> Result<Option<Action>> {
        let Some(result) = &self.results.result else {
            return Ok(Some(Action::Render));
        };

        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let path = PathBuf::from(home).join(format!("sz_export_{timestamp}.csv"));

        let mut file = std::fs::File::create(&path)?;

        // Write header (with CSV escaping)
        let header: Vec<String> = result
            .columns
            .iter()
            .map(|col| {
                if col.contains(',') || col.contains('"') || col.contains('\n') {
                    format!("\"{}\"", col.replace('"', "\"\""))
                } else {
                    col.clone()
                }
            })
            .collect();
        writeln!(file, "{}", header.join(","))?;

        // Write rows
        for row in &result.rows {
            let line: Vec<String> = row
                .iter()
                .map(|v| {
                    let s = format_csv_value(v);
                    if s.contains(',') || s.contains('"') || s.contains('\n') {
                        format!("\"{}\"", s.replace('"', "\"\""))
                    } else {
                        s
                    }
                })
                .collect();
            writeln!(file, "{}", line.join(","))?;
        }

        Ok(Some(Action::StatusMessage(format!(
            "Exported to {}",
            path.display()
        ))))
    }

    pub fn set_focus_editor(&mut self) {
        self.focus = QueryFocus::Editor;
        self.update_focus_state();
    }

    pub fn set_focus_results(&mut self) {
        self.focus = QueryFocus::Results;
        self.update_focus_state();
    }

    /// Advance focus to next sub-pane. Returns true if advanced, false if at last.
    pub fn advance_focus(&mut self) -> bool {
        match self.focus {
            QueryFocus::Editor => {
                self.focus = QueryFocus::Results;
                self.update_focus_state();
                true
            }
            QueryFocus::Results => false,
        }
    }

    /// Retreat focus to previous sub-pane. Returns true if retreated, false if at first.
    pub fn retreat_focus(&mut self) -> bool {
        match self.focus {
            QueryFocus::Results => {
                self.focus = QueryFocus::Editor;
                self.update_focus_state();
                true
            }
            QueryFocus::Editor => false,
        }
    }

    /// Check if we're currently on a chart tab.
    fn is_on_chart_tab(&self) -> bool {
        matches!(self.bottom_tab, BottomTab::Chart(_))
    }

    /// Create a new chart tab. Returns true if created.
    fn create_new_chart_tab(&mut self) -> bool {
        if self.chart_tabs.len() >= chart::MAX_CHART_TABS {
            return false;
        }
        let next_num = self.chart_tabs.len() + 1;
        self.chart_tabs.push(ChartTab::new(next_num));
        let new_index = self.chart_tabs.len() - 1;
        self.bottom_tab = BottomTab::Chart(new_index);
        // Load the new chart tab into the ChartPane
        self.sync_chart_pane_to_tab();
        true
    }

    /// Delete the active chart tab. Returns true if deleted.
    fn delete_active_chart_tab(&mut self) -> bool {
        if let BottomTab::Chart(i) = self.bottom_tab {
            self.chart_tabs.remove(i);
            if self.chart_tabs.is_empty() {
                self.bottom_tab = BottomTab::Results;
            } else {
                let new_index = if i >= self.chart_tabs.len() {
                    self.chart_tabs.len() - 1
                } else {
                    i
                };
                self.bottom_tab = BottomTab::Chart(new_index);
                self.sync_chart_pane_to_tab();
            }
            true
        } else {
            false
        }
    }

    /// Sync `ChartPane` state when switching chart tabs.
    /// Saves current `ChartPane` state back to the old tab config, then loads new tab.
    fn sync_chart_pane_to_tab(&mut self) {
        // Load settings_visible from the active tab
        if let BottomTab::Chart(i) = self.bottom_tab
            && let Some(tab) = self.chart_tabs.get(i)
        {
            self.chart.settings_visible = !tab.settings_hidden;
            // Reset focus to chart display when switching tabs
            self.chart.focus = chart::ChartFocus::ChartDisplay;
            // Auto-prepare chart if config is valid
            self.chart.try_prepare(&tab.config);
        }
    }

    /// Save the current `ChartPane`'s `settings_visible` state back to the active tab.
    fn save_chart_pane_state(&mut self) {
        if let BottomTab::Chart(i) = self.bottom_tab
            && let Some(tab) = self.chart_tabs.get_mut(i)
        {
            tab.settings_hidden = !self.chart.settings_visible;
        }
    }

    fn render_delete_confirm(&self, frame: &mut ratatui::Frame, area: Rect) {
        use ratatui::layout::Flex;
        use ratatui::style::Style;
        use ratatui::text::{Line, Span};
        use ratatui::widgets::{Block, Borders, Clear, Paragraph};

        let label = if let BottomTab::Chart(i) = self.bottom_tab {
            self.chart_tabs
                .get(i)
                .map_or("chart", |t| t.label.as_str())
        } else {
            "chart"
        };

        let msg = format!("Delete \"{label}\"? (Y/N)");
        let width = (msg.chars().count() as u16 + 6).min(area.width.saturating_sub(4));
        let height = 3u16;

        let vert = Layout::vertical([Constraint::Length(height)])
            .flex(Flex::Center)
            .split(area);
        let popup_area = Layout::horizontal([Constraint::Length(width)])
            .flex(Flex::Center)
            .split(vert[0])[0];

        frame.render_widget(Clear, popup_area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(crate::theme::dialog_border()))
            .style(
                Style::default()
                    .bg(crate::theme::dialog_bg())
                    .fg(crate::theme::fg()),
            );
        let inner = block.inner(popup_area);
        frame.render_widget(block, popup_area);

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Delete ", Style::default().fg(crate::theme::fg())),
                Span::styled(
                    format!("\"{label}\""),
                    Style::default().fg(crate::theme::red()),
                ),
                Span::styled("? (Y/N)", Style::default().fg(crate::theme::fg())),
            ]))
            .alignment(ratatui::layout::Alignment::Center)
            .style(Style::default().bg(crate::theme::dialog_bg())),
            inner,
        );
    }

    /// Handle key events when in title edit mode.
    #[allow(clippy::unnecessary_wraps)]
    fn handle_title_edit_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        if let Some((chart_idx, ref mut buf, ref original)) = self.title_edit {
            match key.code {
                KeyCode::Enter => {
                    let new_label = buf.clone();
                    if let Some(tab) = self.chart_tabs.get_mut(chart_idx) {
                        tab.label = if new_label.is_empty() {
                            original.clone()
                        } else {
                            new_label
                        };
                    }
                    self.title_edit = None;
                }
                KeyCode::Esc => {
                    let orig = original.clone();
                    if let Some(tab) = self.chart_tabs.get_mut(chart_idx) {
                        tab.label = orig;
                    }
                    self.title_edit = None;
                }
                KeyCode::Backspace => {
                    buf.pop();
                    if let Some(tab) = self.chart_tabs.get_mut(chart_idx) {
                        tab.label = if buf.is_empty() {
                            original.clone()
                        } else {
                            buf.clone()
                        };
                    }
                }
                KeyCode::Char(c) => {
                    if buf.chars().count() < 20 {
                        buf.push(c);
                        if let Some(tab) = self.chart_tabs.get_mut(chart_idx) {
                            tab.label.clone_from(buf);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(Some(Action::Render))
    }

    /// Format a chart tab label with its sequential number prefix.
    fn chart_tab_display_label(&self, idx: usize) -> String {
        let num = idx + 1;
        let label = &self.chart_tabs[idx].label;
        format!(" {num}: {label} ")
    }
}

impl Component for QueryPane {
    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        // History overlay captures all input when visible
        if self.history.visible {
            return self.history.handle_key(key);
        }

        // Title edit mode captures all input
        if self.title_edit.is_some() {
            return self.handle_title_edit_key(key);
        }

        // Delete chart confirmation captures all input
        if self.delete_confirm {
            match key.code {
                KeyCode::Char('y' | 'Y') => {
                    self.delete_confirm = false;
                    self.delete_active_chart_tab();
                }
                _ => {
                    self.delete_confirm = false;
                }
            }
            return Ok(Some(Action::Render));
        }

        // If a chart popup is open, route to chart
        if self.is_on_chart_tab()
            && self.chart.settings.popup_open.is_some()
            && let BottomTab::Chart(i) = self.bottom_tab
            && let Some(tab) = self.chart_tabs.get_mut(i)
        {
            let result = self.chart.handle_settings_key(key, &mut tab.config);
            self.save_chart_pane_state();
            return result;
        }

        // If nvim is not available, only allow basic navigation
        if !self.nvim_available {
            if key.code == KeyCode::Esc {
                return Ok(None);
            }
            return Ok(Some(Action::Render));
        }

        // Row detail popup on results table captures Esc
        if key.code == KeyCode::Esc
            && self.focus == QueryFocus::Results
            && matches!(self.bottom_tab, BottomTab::Results)
            && self.results.row_detail.visible
        {
            self.results.row_detail.handle_key(key);
            return Ok(Some(Action::Render));
        }

        // Esc handling
        if key.code == KeyCode::Esc {
            // If chart settings are focused, let ChartPane handle Esc (unfocus settings)
            if self.focus == QueryFocus::Results && self.is_on_chart_tab() {
                if self.chart.focus == chart::ChartFocus::Settings {
                    self.chart.focus = chart::ChartFocus::ChartDisplay;
                    return Ok(Some(Action::Render));
                }
                // Chart display focused -- bubble up to app
                return Ok(None);
            }
            if self.focus != QueryFocus::Editor {
                return Ok(None);
            }
            let was_normal = self.editor.is_normal_mode();
            // Always forward Esc to nvim (harmless in normal mode, needed for insert->normal)
            self.editor.handle_key(key)?;
            if was_normal {
                // Already in normal mode -- let app.rs move focus to top bar
                return Ok(None);
            }
            // Was in insert/visual mode -- Esc consumed by nvim for mode change
            return Ok(Some(Action::Render));
        }

        // Letter-based tab switching when in results pane
        if self.focus == QueryFocus::Results {
            // Chart-specific keys when on a chart tab with chart display focused
            if let BottomTab::Chart(_) = self.bottom_tab
                && self.chart.focus == chart::ChartFocus::ChartDisplay
                && let KeyCode::Char('D' | 'd') = key.code
            {
                // D: confirm delete chart tab
                self.delete_confirm = true;
                return Ok(Some(Action::Render));
            }

            // Tab switching only when chart settings are NOT focused.
            let settings_focused =
                self.is_on_chart_tab() && self.chart.focus == chart::ChartFocus::Settings;

            if !settings_focused {
                match key.code {
                    KeyCode::Char('t' | 'T') => {
                        // T: switch to Table tab
                        self.save_chart_pane_state();
                        self.bottom_tab = BottomTab::Results;
                        return Ok(Some(Action::Render));
                    }
                    KeyCode::Char(ch @ '1'..='9') => {
                        // Number keys switch chart tabs (1-indexed)
                        let idx = (ch as usize) - ('1' as usize);
                        if idx < self.chart_tabs.len() {
                            self.save_chart_pane_state();
                            self.bottom_tab = BottomTab::Chart(idx);
                            self.sync_chart_pane_to_tab();
                            return Ok(Some(Action::Render));
                        }
                    }
                    _ => {}
                }
            }
        }

        // Ctrl+Up/Down: resize split; Ctrl+C: toggle chart settings
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Up => {
                    if self.split_pct > 15 {
                        self.split_pct -= 5;
                    }
                    return Ok(Some(Action::Render));
                }
                KeyCode::Down => {
                    if self.split_pct < 85 {
                        self.split_pct += 5;
                    }
                    return Ok(Some(Action::Render));
                }
                KeyCode::Char('c') => {
                    if self.focus == QueryFocus::Results && self.is_on_chart_tab() {
                        self.chart.toggle_settings();
                        self.save_chart_pane_state();
                        return Ok(Some(Action::Render));
                    }
                }
                KeyCode::Char('n') => {
                    if self.focus == QueryFocus::Results {
                        self.create_new_chart_tab();
                        return Ok(Some(Action::Render));
                    }
                }
                KeyCode::Char('e') => {
                    if self.focus == QueryFocus::Results
                        && let BottomTab::Chart(i) = self.bottom_tab
                        && let Some(tab) = self.chart_tabs.get(i)
                    {
                        let current = tab.label.clone();
                        self.title_edit = Some((i, current.clone(), current));
                        return Ok(Some(Action::Render));
                    }
                }
                KeyCode::Char('s') => {
                    if self.focus == QueryFocus::Results {
                        return Ok(Some(Action::SaveQueryToHistory));
                    }
                }
                _ => {}
            }
        }

        // Route to focused component
        match self.focus {
            QueryFocus::Editor => self.editor.handle_key(key),
            QueryFocus::Results => match self.bottom_tab {
                BottomTab::Results => self.results.handle_key(key),
                BottomTab::Chart(i) => {
                    if let Some(tab) = self.chart_tabs.get_mut(i) {
                        let result = self.chart.handle_key(key, &mut tab.config);
                        self.save_chart_pane_state();
                        result
                    } else {
                        Ok(None)
                    }
                }
            },
        }
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::NewQuery => {
                self.editor.set_sql("");
                self.results.clear();
                self.chart.clear();
                self.chart_tabs.clear();
                self.bottom_tab = BottomTab::Results;
                self.focus = QueryFocus::Editor;
                self.update_focus_state();
                // Propagate up so app.rs can clear loaded_saved_query_id
                Ok(Some(Action::NewQuery))
            }
            Action::ExecuteQuery(_) => {
                self.results.set_loading();
                self.chart.clear();
                self.bottom_tab = BottomTab::Results;
                // The action propagates up to App which spawns the async task
                Ok(Some(action))
            }
            Action::QueryResultData(result) => {
                self.chart.on_new_result(&result);
                // Auto-generate chart for the active chart tab if it has a valid config
                if let BottomTab::Chart(i) = self.bottom_tab
                    && let Some(tab) = self.chart_tabs.get(i)
                {
                    self.chart.try_prepare(&tab.config);
                }
                let owned = std::sync::Arc::try_unwrap(result).unwrap_or_else(|a| (*a).clone());
                self.results.set_result(owned);
                self.focus = QueryFocus::Results;
                self.update_focus_state();
                Ok(Some(Action::Render))
            }
            Action::QueryError(err) => {
                self.results.set_error(err);
                Ok(Some(Action::Render))
            }
            Action::ShowQueryHistory => {
                self.history.show(self.query_history_entries.clone());
                Ok(Some(Action::Render))
            }
            Action::LoadQueryFromHistory(sql) => {
                self.editor.set_sql(&sql);
                self.focus = QueryFocus::Editor;
                self.update_focus_state();
                Ok(Some(Action::Render))
            }
            Action::ExportResults => match self.export_csv() {
                Ok(action) => Ok(action),
                Err(e) => Ok(Some(Action::StatusMessage(format!("Export failed: {e}")))),
            },
            Action::NvimExited(ref msg) => {
                tracing::error!("Nvim exited: {}", msg);
                self.editor.handle_nvim_exited(msg);
                Ok(None)
            }
            Action::NvimReady => {
                tracing::info!("Nvim is ready");
                // NvimReady is handled by app.rs dispatch which starts the RPC connect task
                Ok(Some(Action::Render))
            }
            Action::NvimRpcConnected => {
                tracing::info!("Nvim RPC connected");
                Ok(Some(Action::Render))
            }
            Action::StatusMessage(msg) => {
                self.show_status(msg);
                Ok(Some(Action::Render))
            }
            Action::ClearStatus => {
                self.status_message = None;
                Ok(Some(Action::Render))
            }
            Action::SetQueryHistory(entries) => {
                self.set_history(entries);
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Sync sub-component focus flags from app_focused (set by app.rs just before render)
        self.update_focus_state();

        // If nvim is not available, show error
        if !self.nvim_available {
            let error_msg = self
                .nvim_error
                .as_deref()
                .unwrap_or("nvim not found on PATH");
            let block = ratatui::widgets::Block::default()
                .borders(ratatui::widgets::Borders::ALL)
                .border_style(ratatui::style::Style::default().fg(crate::theme::red()))
                .title(" Query Mode - Error ")
                .title_style(ratatui::style::Style::default().fg(crate::theme::accent_dim()))
                .style(
                    ratatui::style::Style::default()
                        .bg(crate::theme::bg())
                        .fg(crate::theme::fg()),
                );

            let error_text = ratatui::widgets::Paragraph::new(format!(
                "\n  Query mode requires neovim (nvim) to be installed.\n\n  {error_msg}\n\n  Install neovim: https://neovim.io/\n  Then restart sz."
            ))
            .block(block);
            frame.render_widget(error_text, area);
            return;
        }

        // Split into editor (top) and results (bottom)
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(self.split_pct),
                Constraint::Percentage(100 - self.split_pct),
            ])
            .split(area);

        // Render editor (needs &mut self)
        self.editor.render(frame, chunks[0]);

        // Build the tab bar with Table + chart tabs
        let bottom_focused = self.app_focused && self.focus == QueryFocus::Results;
        let mut tab_spans: Vec<ratatui::text::Span> = Vec::new();

        // Leading space
        tab_spans.push(ratatui::text::Span::styled(
            " ",
            ratatui::style::Style::default().bg(crate::theme::bg_surface()),
        ));

        // Table tab
        let table_active = matches!(self.bottom_tab, BottomTab::Results);
        tab_spans.push(ratatui::text::Span::styled(
            " Table ",
            if table_active {
                ratatui::style::Style::default()
                    .fg(crate::theme::fg_bright())
                    .bg(crate::theme::tab_active_bg())
                    .add_modifier(ratatui::style::Modifier::BOLD)
            } else {
                ratatui::style::Style::default()
                    .fg(crate::theme::fg_dim())
                    .bg(crate::theme::tab_inactive_bg())
            },
        ));

        // Chart tabs
        for (idx, _chart_tab) in self.chart_tabs.iter().enumerate() {
            // Separator space
            tab_spans.push(ratatui::text::Span::styled(
                " ",
                ratatui::style::Style::default().bg(crate::theme::bg_surface()),
            ));

            let is_active = matches!(self.bottom_tab, BottomTab::Chart(i) if i == idx);

            let label = if let Some((edit_idx, ref buf, _)) = self.title_edit {
                if edit_idx == idx {
                    format!(" {}: {}| ", idx + 1, buf)
                } else {
                    self.chart_tab_display_label(idx)
                }
            } else {
                self.chart_tab_display_label(idx)
            };

            tab_spans.push(ratatui::text::Span::styled(
                label,
                if is_active {
                    ratatui::style::Style::default()
                        .fg(crate::theme::fg_bright())
                        .bg(crate::theme::tab_active_bg())
                        .add_modifier(ratatui::style::Modifier::BOLD)
                } else {
                    ratatui::style::Style::default()
                        .fg(crate::theme::fg_dim())
                        .bg(crate::theme::tab_inactive_bg())
                },
            ));
        }

        // Status message (auto-clear after 3 seconds)
        if let Some((ref msg, instant)) = self.status_message {
            if instant.elapsed().as_secs() < 3 {
                tab_spans.push(ratatui::text::Span::styled(
                    format!("  {msg}"),
                    ratatui::style::Style::default().fg(crate::theme::green()),
                ));
            } else {
                // Will be cleared after borrow ends
            }
        }
        if self
            .status_message
            .as_ref()
            .is_some_and(|(_, i)| i.elapsed().as_secs() >= 3)
        {
            self.status_message = None;
        }

        let tab_bar = ratatui::text::Line::from(tab_spans);

        // Render the Output pane border
        let border_color = if bottom_focused {
            crate::theme::border_focus()
        } else {
            crate::theme::border()
        };

        let block = ratatui::widgets::Block::default()
            .borders(ratatui::widgets::Borders::ALL)
            .border_style(ratatui::style::Style::default().fg(border_color))
            .title(" Results ")
            .title_style(ratatui::style::Style::default().fg(crate::theme::accent_dim()))
            .style(
                ratatui::style::Style::default()
                    .bg(crate::theme::bg_surface())
                    .fg(crate::theme::fg()),
            );

        let inner = block.inner(chunks[1]);
        frame.render_widget(block, chunks[1]);

        if inner.height >= 2 && inner.width >= 4 {
            // Split inner: top padding (1) + tab bar (1) + separator (1) + content (rest)
            let inner_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // top padding
                    Constraint::Length(1), // tab bar
                    Constraint::Length(1), // separator
                    Constraint::Min(0),    // content
                ])
                .split(inner);

            // Tab bar
            let tab_paragraph = ratatui::widgets::Paragraph::new(tab_bar).style(
                ratatui::style::Style::default()
                    .bg(crate::theme::bg_surface())
                    .fg(crate::theme::fg()),
            );
            frame.render_widget(tab_paragraph, inner_chunks[1]);

            // Separator
            let sep = "\u{2500}".repeat(inner_chunks[2].width as usize);
            let sep_line = ratatui::widgets::Paragraph::new(ratatui::text::Line::from(
                ratatui::text::Span::styled(
                    sep,
                    ratatui::style::Style::default().fg(crate::theme::border()),
                ),
            ))
            .style(
                ratatui::style::Style::default()
                    .bg(crate::theme::bg_surface())
                    .fg(crate::theme::fg()),
            );
            frame.render_widget(sep_line, inner_chunks[2]);

            // Content area
            let content_area = inner_chunks[3];
            match self.bottom_tab {
                BottomTab::Results => {
                    self.results.render_content(frame, content_area);
                    // Render row detail popup on top of output area, centered on full screen
                    self.results.row_detail.render(frame, inner, frame.area());
                }
                BottomTab::Chart(i) => {
                    if let Some(tab) = self.chart_tabs.get(i) {
                        let config = tab.config.clone();
                        self.chart.render_content(frame, content_area, &config);
                        // Render popup on top of the full output area (inner)
                        self.chart.render_popup(frame, inner, &config);
                    }
                }
            }
        }

        // Render delete confirmation popup
        if self.delete_confirm {
            self.render_delete_confirm(frame, area);
        }

        // Render history overlay on top of everything
        self.history.render(frame, area);
    }
}

fn format_csv_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}
