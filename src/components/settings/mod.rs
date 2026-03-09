#![allow(clippy::cast_possible_truncation, clippy::cast_precision_loss, clippy::cast_sign_loss, clippy::cast_lossless, clippy::too_many_lines)]

mod theme_pane;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use color_eyre::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Flex, Layout, Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table};

use crate::action::Action;
use crate::components::Component;
use crate::config::SnowflakeConnection;
use crate::theme;
use theme_pane::ThemeEditorPane;

/// How long status messages remain visible (seconds)
const STATUS_TIMEOUT_SECS: u64 = 3;

/// Which pane is focused
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsFocus {
    Connection,
    Cache,
    Theme,
}

/// Which field is currently focused within the Connection pane
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnField {
    Connection,
    ApplyButton,
}

impl ConnField {
    fn all() -> &'static [ConnField] {
        &[ConnField::Connection, ConnField::ApplyButton]
    }

    fn index(self) -> usize {
        ConnField::all()
            .iter()
            .position(|&f| f == self)
            .unwrap_or(0)
    }

    fn from_index(idx: usize) -> Self {
        let fields = ConnField::all();
        fields[idx.min(fields.len() - 1)]
    }
}

/// Which field is currently focused within the Cache pane
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CacheField {
    ClearCacheButton,
    RefreshNowButton,
}

impl CacheField {
    fn all() -> &'static [CacheField] {
        &[CacheField::ClearCacheButton, CacheField::RefreshNowButton]
    }

    fn index(self) -> usize {
        CacheField::all()
            .iter()
            .position(|&f| f == self)
            .unwrap_or(0)
    }

    fn from_index(idx: usize) -> Self {
        let fields = CacheField::all();
        fields[idx.min(fields.len() - 1)]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputMode {
    Normal,
    PopupOpen,
}

pub struct SettingsPane {
    /// Whether this pane has focus from the app
    pub app_focused: bool,
    /// Which pane is focused
    pub focus: SettingsFocus,
    /// Input mode
    input_mode: InputMode,

    /// Current focused field within connection pane
    conn_field: ConnField,
    /// Current focused field within cache pane
    cache_field: CacheField,

    /// All available connection names (from config)
    connection_names: Vec<String>,
    /// Connection details keyed by name
    connections: HashMap<String, SnowflakeConnection>,
    /// Currently selected connection index
    selected_connection_idx: usize,
    /// Popup selection index (for connection picker)
    popup_index: usize,

    /// The active (applied) connection name
    active_connection: Option<String>,

    /// Cache info
    cache_dir: PathBuf,
    cache_last_refreshed: Option<String>,
    cache_size_bytes: u64,

    /// Catalog refresh progress (mirrors `tree.refresh_running` / `refresh_total`)
    pub refresh_running: usize,
    pub refresh_total: usize,

    /// Status message for the connection pane (with auto-clear timestamp)
    conn_status: Option<(String, Instant)>,
    /// Status message for the cache pane (with auto-clear timestamp)
    cache_status: Option<(String, Instant)>,

    /// Theme editor pane
    theme_pane: ThemeEditorPane,
}

impl SettingsPane {
    pub fn new() -> Self {
        Self {
            app_focused: false,
            focus: SettingsFocus::Connection,
            input_mode: InputMode::Normal,
            conn_field: ConnField::Connection,
            cache_field: CacheField::ClearCacheButton,
            connection_names: Vec::new(),
            connections: HashMap::new(),
            selected_connection_idx: 0,
            popup_index: 0,
            active_connection: None,
            cache_dir: PathBuf::new(),
            cache_last_refreshed: None,
            cache_size_bytes: 0,
            refresh_running: 0,
            refresh_total: 0,
            conn_status: None,
            cache_status: None,
            theme_pane: ThemeEditorPane::new(),
        }
    }

    /// Initialize with config data. Called from main.rs after construction.
    pub fn init_from_config(
        &mut self,
        connections: HashMap<String, SnowflakeConnection>,
        default_connection: Option<String>,
        cache_dir: PathBuf,
    ) {
        let mut names: Vec<String> = connections.keys().cloned().collect();
        names.sort();

        self.connections = connections;
        self.connection_names = names;

        // Determine the active connection: explicit default, or fall back to
        // a connection named "default", or fall back to the first connection.
        let resolved = default_connection
            .or_else(|| {
                self.connection_names
                    .iter()
                    .find(|n| n.as_str() == "default")
                    .cloned()
            })
            .or_else(|| self.connection_names.first().cloned());

        if let Some(ref name) = resolved {
            self.selected_connection_idx = self
                .connection_names
                .iter()
                .position(|n| n == name)
                .unwrap_or(0);
            self.active_connection = Some(name.clone());
        }

        self.cache_dir = cache_dir;
        self.refresh_cache_info();
        self.theme_pane.load_from_config(theme::current_config());
    }

    fn current_connection_name(&self) -> Option<&str> {
        self.connection_names
            .get(self.selected_connection_idx)
            .map(std::string::String::as_str)
    }

    fn current_connection(&self) -> Option<&SnowflakeConnection> {
        self.current_connection_name()
            .and_then(|name| self.connections.get(name))
    }

    fn refresh_cache_info(&mut self) {
        self.cache_size_bytes = 0;
        if let Ok(entries) = std::fs::read_dir(&self.cache_dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    self.cache_size_bytes += meta.len();
                }
            }
        }

        let key = self
            .active_connection
            .as_deref()
            .unwrap_or("_default");
        let catalog_path = self.cache_dir.join(format!("catalog_{key}.bin"));
        if let Ok(meta) = std::fs::metadata(&catalog_path)
            && let Ok(modified) = meta.modified()
        {
            let datetime: chrono::DateTime<chrono::Local> = modified.into();
            self.cache_last_refreshed = Some(datetime.format("%Y-%m-%d %H:%M:%S").to_string());
        }
    }

    fn format_cache_size(&self) -> String {
        let bytes = self.cache_size_bytes;
        if bytes < 1024 {
            format!("{bytes} B")
        } else if bytes < 1024 * 1024 {
            format!("{:.1} KB", bytes as f64 / 1024.0)
        } else {
            format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
        }
    }

    /// Get a live status message, returning None if it has expired.
    fn active_status(status: Option<&(String, Instant)>) -> Option<&str> {
        if let Some((msg, ts)) = status
            && ts.elapsed().as_secs() < STATUS_TIMEOUT_SECS
        {
            return Some(msg.as_str());
        }
        None
    }

    fn open_connection_popup(&mut self) {
        if !self.connection_names.is_empty() {
            self.input_mode = InputMode::PopupOpen;
            self.popup_index = self.selected_connection_idx;
        }
    }

    // --- Key handlers ---

    fn handle_conn_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        if self.input_mode == InputMode::PopupOpen {
            return self.handle_popup_key(key);
        }

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                let fields = ConnField::all();
                let idx = self.conn_field.index();
                self.conn_field = ConnField::from_index((idx + 1) % fields.len());
                Ok(Some(Action::Render))
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let fields = ConnField::all();
                let idx = self.conn_field.index();
                self.conn_field = ConnField::from_index((idx + fields.len() - 1) % fields.len());
                Ok(Some(Action::Render))
            }
            KeyCode::Enter | KeyCode::Char(' ') => self.activate_conn_field(),
            KeyCode::Char('l') | KeyCode::Right => {
                if self.conn_field == ConnField::Connection {
                    self.open_connection_popup();
                    return Ok(Some(Action::Render));
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    #[allow(clippy::unnecessary_wraps)]
    fn activate_conn_field(&mut self) -> Result<Option<Action>> {
        match self.conn_field {
            ConnField::Connection => {
                self.open_connection_popup();
                Ok(Some(Action::Render))
            }
            ConnField::ApplyButton => {
                let conn_name = self.current_connection_name().map(std::string::ToString::to_string);
                self.active_connection.clone_from(&conn_name);
                self.conn_status = Some(("Connection applied".to_string(), Instant::now()));
                Ok(Some(Action::ApplyOverrides {
                    connection_name: conn_name,
                }))
            }
        }
    }

    #[allow(clippy::unnecessary_wraps)]
    fn handle_popup_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        let count = self.connection_names.len();
        if count == 0 {
            self.input_mode = InputMode::Normal;
            return Ok(Some(Action::Render));
        }

        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                Ok(Some(Action::Render))
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if self.popup_index + 1 < count {
                    self.popup_index += 1;
                }
                Ok(Some(Action::Render))
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.popup_index = self.popup_index.saturating_sub(1);
                Ok(Some(Action::Render))
            }
            KeyCode::Enter => {
                self.selected_connection_idx = self.popup_index;
                self.input_mode = InputMode::Normal;
                Ok(Some(Action::Render))
            }
            _ => Ok(None),
        }
    }

    #[allow(clippy::unnecessary_wraps)]
    fn handle_cache_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        match key.code {
            KeyCode::Char('l') | KeyCode::Right => {
                let fields = CacheField::all();
                let idx = self.cache_field.index();
                self.cache_field = CacheField::from_index((idx + 1) % fields.len());
                Ok(Some(Action::Render))
            }
            KeyCode::Char('h') | KeyCode::Left => {
                let fields = CacheField::all();
                let idx = self.cache_field.index();
                self.cache_field = CacheField::from_index((idx + fields.len() - 1) % fields.len());
                Ok(Some(Action::Render))
            }
            KeyCode::Enter | KeyCode::Char(' ') => match self.cache_field {
                CacheField::ClearCacheButton => {
                    self.cache_status = Some(("Clearing cache...".to_string(), Instant::now()));
                    Ok(Some(Action::ClearCache))
                }
                CacheField::RefreshNowButton => {
                    self.cache_status = Some(("Refreshing catalog...".to_string(), Instant::now()));
                    Ok(Some(Action::RefreshCatalog))
                }
            },
            _ => Ok(None),
        }
    }

    // --- Render helpers ---

    /// Render a compact field: "Label: Value [▶]" on one line, matching chart settings style.
    fn render_compact_field(
        frame: &mut Frame,
        x: u16,
        y: u16,
        w: u16,
        label: &str,
        value: &str,
        focused: bool,
    ) {
        let label_width = 14;
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

        let padded_label = format!("{label:<label_width$}");
        let truncated_val: String = value.chars().take(val_max_len).collect();

        let suffix = if focused { " \u{25B6}" } else { "" };

        let val_with_suffix: String = if truncated_val.chars().count() + suffix.len() > val_max_len
        {
            let t: String = truncated_val
                .chars()
                .take(val_max_len.saturating_sub(suffix.len()))
                .collect();
            format!("{t}{suffix}")
        } else {
            format!("{truncated_val}{suffix}")
        };

        let spans = vec![
            Span::styled(padded_label, label_style),
            Span::styled(colon, Style::default().fg(theme::fg_dim())),
            Span::styled(val_with_suffix, val_style),
        ];

        let line = Line::from(spans);
        let field_area = Rect::new(x, y, w, 1);
        frame.render_widget(
            Paragraph::new(line).style(Style::default().bg(theme::bg_surface())),
            field_area,
        );
    }

    /// Render a read-only info field: "Label: Value" (never highlighted).
    fn render_info_field(frame: &mut Frame, x: u16, y: u16, w: u16, label: &str, value: &str) {
        let label_width = 14;
        let colon = ": ";
        let prefix_len = label_width + colon.len();
        let val_max_len = (w as usize).saturating_sub(prefix_len);

        let padded_label = format!("{label:<label_width$}");
        let truncated_val: String = value.chars().take(val_max_len).collect();

        let spans = vec![
            Span::styled(padded_label, Style::default().fg(theme::fg_dim())),
            Span::styled(colon, Style::default().fg(theme::fg_dim())),
            Span::styled(truncated_val, Style::default().fg(theme::fg())),
        ];

        let line = Line::from(spans);
        let field_area = Rect::new(x, y, w, 1);
        frame.render_widget(
            Paragraph::new(line).style(Style::default().bg(theme::bg_surface())),
            field_area,
        );
    }

    fn render_button(label: &str, focused: bool) -> Paragraph<'_> {
        let style = if focused {
            Style::default()
                .fg(theme::fg_bright())
                .bg(theme::accent_dim())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::fg()).bg(theme::bg_highlight())
        };
        Paragraph::new(format!(" {label} ")).style(style)
    }

    fn render_connection_popup(&self, frame: &mut Frame, area: Rect) {
        if self.input_mode != InputMode::PopupOpen || self.connection_names.is_empty() {
            return;
        }

        let max_name_len = self
            .connection_names
            .iter()
            .map(|n| n.chars().count())
            .max()
            .unwrap_or(10);
        let content_width = max_name_len + 6;
        let popup_width = (content_width as u16 + 4)
            .min(area.width.saturating_sub(4))
            .max(20);
        let popup_height = (self.connection_names.len() as u16 + 4)
            .min(area.height.saturating_sub(4))
            .max(5);

        let popup_area = centered_rect(popup_width, popup_height, area);

        frame.render_widget(Clear, popup_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::dialog_border()))
            .title(" Connection ")
            .title_style(Style::default().fg(theme::accent_dim()))
            .style(Style::default().bg(theme::dialog_bg()).fg(theme::fg()));

        let inner = block.inner(popup_area);
        frame.render_widget(block, popup_area);

        if inner.height < 1 || inner.width < 4 {
            return;
        }

        let visible_items = (inner.height as usize).min(self.connection_names.len());
        let scroll_offset = if self.popup_index >= visible_items {
            self.popup_index - visible_items + 1
        } else {
            0
        };

        let rows: Vec<Row> = self
            .connection_names
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(visible_items)
            .map(|(i, name)| {
                let is_selected = i == self.popup_index;
                let is_active = Some(name.as_str()) == self.active_connection.as_deref();
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

                let marker = if is_active { "* " } else { "  " };
                let cell = Cell::from(format!(" {marker}{name}"))
                    .style(Style::default().fg(fg).bg(bg));
                Row::new(vec![cell])
            })
            .collect();

        let table = Table::new(rows, &[Constraint::Min(4)])
            .column_spacing(0)
            .block(Block::default().style(Style::default().bg(theme::dialog_bg())));

        frame.render_widget(table, inner);
    }

    fn conn_hint_text(&self) -> Line<'_> {
        let dim = Style::default().fg(theme::fg_dim());
        let accent = Style::default().fg(theme::accent_dim());

        match self.input_mode {
            InputMode::PopupOpen => Line::from(vec![
                Span::styled(" j/k", accent),
                Span::styled(": Navigate  ", dim),
                Span::styled("Enter", accent),
                Span::styled(": Select  ", dim),
                Span::styled("Esc", accent),
                Span::styled(": Cancel", dim),
            ]),
            InputMode::Normal => match self.conn_field {
                ConnField::Connection => Line::from(vec![
                    Span::styled(" j/k", accent),
                    Span::styled(": Navigate  ", dim),
                    Span::styled("Enter", accent),
                    Span::styled(": Switch  ", dim),
                    Span::styled("Tab", accent),
                    Span::styled(": Cache", dim),
                ]),
                ConnField::ApplyButton => Line::from(vec![
                    Span::styled(" j/k", accent),
                    Span::styled(": Navigate  ", dim),
                    Span::styled("Enter", accent),
                    Span::styled(": Apply  ", dim),
                    Span::styled("Tab", accent),
                    Span::styled(": Cache", dim),
                ]),
            },
        }
    }

    fn cache_hint_text() -> Line<'static> {
        let dim = Style::default().fg(theme::fg_dim());
        let accent = Style::default().fg(theme::accent_dim());

        Line::from(vec![
            Span::styled(" h/l", accent),
            Span::styled(": Navigate  ", dim),
            Span::styled("Enter", accent),
            Span::styled(": Activate  ", dim),
            Span::styled("Tab", accent),
            Span::styled(": Theme  ", dim),
            Span::styled("Shift+Tab", accent),
            Span::styled(": Connection", dim),
        ])
    }

    fn render_connection_pane(&self, frame: &mut Frame, area: Rect) {
        let focused = self.app_focused && self.focus == SettingsFocus::Connection;
        let border_color = if focused {
            theme::border_focus()
        } else {
            theme::border()
        };

        let block = Block::default()
            .title(" Connection Settings ")
            .title_style(Style::default().fg(theme::accent_dim()))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(theme::bg_surface()).fg(theme::fg()));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.height < 6 || inner.width < 20 {
            return;
        }

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Connection (selectable)
                Constraint::Length(1), // spacer
                Constraint::Length(1), // Warehouse (read-only)
                Constraint::Length(1), // Database (read-only)
                Constraint::Length(1), // Role (read-only)
                Constraint::Length(1), // spacer
                Constraint::Length(1), // Apply button
                Constraint::Min(0),    // status + hint
            ])
            .split(inner.inner(Margin::new(1, 0)));

        let x = inner.x + 1;
        let w = inner.width.saturating_sub(2);

        let conn = self.current_connection();

        // Connection field (selectable)
        {
            let is_focused = focused && self.conn_field == ConnField::Connection;
            let conn_name = self.current_connection_name().unwrap_or("(none)");
            Self::render_compact_field(frame, x, rows[0].y, w, "Connection", conn_name, is_focused);
        }

        // Warehouse (read-only, from config)
        {
            let val = conn
                .and_then(|c| c.warehouse.as_deref())
                .unwrap_or("(not set)");
            Self::render_info_field(frame, x, rows[2].y, w, "Warehouse", val);
        }

        // Database (read-only, from config)
        {
            let val = conn
                .and_then(|c| c.database.as_deref())
                .unwrap_or("(not set)");
            Self::render_info_field(frame, x, rows[3].y, w, "Database", val);
        }

        // Role (read-only, from config)
        {
            let val = conn.and_then(|c| c.role.as_deref()).unwrap_or("(not set)");
            Self::render_info_field(frame, x, rows[4].y, w, "Role", val);
        }

        // Apply button — left-aligned
        {
            frame.render_widget(
                Self::render_button(
                    "Apply",
                    focused && self.conn_field == ConnField::ApplyButton,
                ),
                Rect::new(x, rows[6].y, 9, 1),
            );
        }

        // Status + hint in remaining space
        if rows[7].height >= 1 {
            let bottom = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(rows[7]);

            // Status message (auto-expires)
            if let Some(msg) = Self::active_status(self.conn_status.as_ref()) {
                let status_line = Line::from(Span::styled(
                    format!(" {msg}"),
                    Style::default().fg(theme::green()),
                ));
                frame.render_widget(
                    Paragraph::new(status_line).style(Style::default().bg(theme::bg_surface())),
                    bottom[0],
                );
            }

            // Hint bar
            if focused {
                let hints = self.conn_hint_text();
                frame.render_widget(
                    Paragraph::new(hints).style(Style::default().bg(theme::bg_surface())),
                    bottom[1],
                );
            }
        }
    }

    fn render_cache_pane(&self, frame: &mut Frame, area: Rect) {
        let focused = self.app_focused && self.focus == SettingsFocus::Cache;
        let border_color = if focused {
            theme::border_focus()
        } else {
            theme::border()
        };

        let block = Block::default()
            .title(" Cache ")
            .title_style(Style::default().fg(theme::accent_dim()))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(theme::bg_surface()).fg(theme::fg()));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.height < 4 || inner.width < 20 {
            return;
        }

        // Determine how many extra lines we need for status/progress
        let has_progress = self.refresh_total > 0;
        let has_status = Self::active_status(self.cache_status.as_ref()).is_some();

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),                                // Last refreshed
                Constraint::Length(1),                                // Cache size
                Constraint::Length(u16::from(has_progress)), // Refresh progress
                Constraint::Length(u16::from(has_status)),   // Status message
                Constraint::Length(1),                                // spacer
                Constraint::Length(1),                                // buttons
                Constraint::Min(0),                                   // hint
            ])
            .split(inner.inner(Margin::new(1, 0)));

        let x = inner.x + 1;
        let w = inner.width.saturating_sub(2);

        // Last refreshed (read-only)
        {
            let refreshed = self.cache_last_refreshed.as_deref().unwrap_or("Never");
            Self::render_info_field(frame, x, rows[0].y, w, "Last refresh", refreshed);
        }

        // Cache size (read-only)
        {
            let size = self.format_cache_size();
            Self::render_info_field(frame, x, rows[1].y, w, "Cache size", &size);
        }

        // Refresh progress
        if has_progress {
            let progress_text = format!(
                "Refreshing: {}/{}",
                self.refresh_running, self.refresh_total
            );
            let progress_line = Line::from(Span::styled(
                format!(" {progress_text}"),
                Style::default().fg(theme::yellow()),
            ));
            frame.render_widget(
                Paragraph::new(progress_line).style(Style::default().bg(theme::bg_surface())),
                rows[2],
            );
        }

        // Status message (auto-expires)
        if let Some(msg) = Self::active_status(self.cache_status.as_ref()) {
            let status_line = Line::from(Span::styled(
                format!(" {msg}"),
                Style::default().fg(theme::green()),
            ));
            frame.render_widget(
                Paragraph::new(status_line).style(Style::default().bg(theme::bg_surface())),
                rows[3],
            );
        }

        // Cache buttons — left-aligned
        {
            let btn_layout = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(15),
                    Constraint::Length(2),
                    Constraint::Length(15),
                    Constraint::Min(0),
                ])
                .split(rows[5]);

            frame.render_widget(
                Self::render_button(
                    "Clear Cache",
                    focused && self.cache_field == CacheField::ClearCacheButton,
                ),
                btn_layout[0],
            );
            frame.render_widget(
                Self::render_button(
                    "Refresh Now",
                    focused && self.cache_field == CacheField::RefreshNowButton,
                ),
                btn_layout[2],
            );
        }

        // Hint bar
        if focused && rows[6].height >= 1 {
            let bottom = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(rows[6]);

            let hints = Self::cache_hint_text();
            frame.render_widget(
                Paragraph::new(hints).style(Style::default().bg(theme::bg_surface())),
                bottom[1],
            );
        }
    }
}

impl Component for SettingsPane {
    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        // Popup mode captures all input in the connection pane
        if self.focus == SettingsFocus::Connection && self.input_mode == InputMode::PopupOpen {
            return self.handle_conn_key(key);
        }

        // Esc: bubble up (app.rs handles focus).
        // But Theme pane might consume Esc when editing.
        if key.code == KeyCode::Esc {
            if self.focus == SettingsFocus::Theme {
                return self.theme_pane.handle_key(key);
            }
            return Ok(None);
        }

        match self.focus {
            SettingsFocus::Connection => self.handle_conn_key(key),
            SettingsFocus::Cache => self.handle_cache_key(key),
            SettingsFocus::Theme => self.theme_pane.handle_key(key),
        }
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::CacheClearDone => {
                self.refresh_cache_info();
                self.cache_status =
                    Some(("Cache cleared successfully".to_string(), Instant::now()));
                Ok(Some(Action::Render))
            }
            Action::CatalogLoaded(_, _, _, _) => {
                self.refresh_cache_info();
                self.cache_status = Some(("Catalog refreshed".to_string(), Instant::now()));
                Ok(Some(Action::Render))
            }
            Action::ThemeSaved => self.theme_pane.update(action),
            _ => Ok(None),
        }
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Three-pane vertical layout: Connection (fixed) + Cache (fixed) + Theme (fills)
        let cache_extra = {
            let has_progress = self.refresh_total > 0;
            let has_status = Self::active_status(self.cache_status.as_ref()).is_some();
            u16::from(has_progress) + u16::from(has_status)
        };
        let cache_height = 8 + cache_extra;

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(12),         // Connection (fixed)
                Constraint::Length(cache_height), // Cache (fixed)
                Constraint::Min(10),            // Theme (fills remaining)
            ])
            .split(area);

        self.render_connection_pane(frame, chunks[0]);
        self.render_cache_pane(frame, chunks[1]);
        self.theme_pane.render(
            frame,
            chunks[2],
            self.app_focused && self.focus == SettingsFocus::Theme,
        );

        // Connection popup overlay (on top of everything)
        self.render_connection_popup(frame, area);
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
