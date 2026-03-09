pub mod config;
pub mod prepare;
pub mod render;
pub mod settings;
pub mod types;

use color_eyre::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::action::Action;
use crate::snowflake::QueryResult;

use self::config::ChartConfig;
use self::settings::ChartSettings;
use self::types::PreparedChart;

pub const MAX_CHART_TABS: usize = 9;
const SETTINGS_WIDTH: u16 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartFocus {
    ChartDisplay,
    Settings,
}

/// `ChartPane` renders a single chart with optional settings panel.
/// Tab management (multiple charts) is handled by `QueryPane`.
pub struct ChartPane {
    pub prepared: Option<Result<PreparedChart, String>>,
    result: Option<QueryResult>,
    pub settings: ChartSettings,
    pub focus: ChartFocus,
    pub settings_visible: bool,
}

impl ChartPane {
    pub fn new() -> Self {
        Self {
            prepared: None,
            result: None,
            settings: ChartSettings::new(),
            focus: ChartFocus::ChartDisplay,
            settings_visible: false,
        }
    }

    pub fn on_new_result(&mut self, result: &QueryResult) {
        self.result = Some(result.clone());
        self.settings.update_column_types(result);
    }

    /// Try to generate a chart from current result and given config.
    /// Returns true if a chart was generated (or an error was produced).
    pub fn try_prepare(&mut self, config: &ChartConfig) -> bool {
        if let Some(ref result) = self.result
            && config.x_column.is_some()
            && config.y_column.is_some()
        {
            self.prepared = Some(prepare::prepare(result, config));
            return true;
        }
        self.prepared = None;
        false
    }

    pub fn clear(&mut self) {
        self.result = None;
        self.settings.clear_column_types();
        self.prepared = None;
    }

    /// Handle key when settings are focused. Returns the settings key result.
    #[allow(clippy::unnecessary_wraps)]
    pub fn handle_settings_key(
        &mut self,
        key: KeyEvent,
        config: &mut ChartConfig,
    ) -> Result<Option<Action>> {
        match self.settings.handle_key(key, config) {
            Some(true) => {
                // Generate was activated
                if let Some(ref result) = self.result {
                    self.prepared = Some(prepare::prepare(result, config));
                }
                Ok(Some(Action::Render))
            }
            Some(false) => {
                // Key consumed, config updated by caller
                Ok(Some(Action::Render))
            }
            None => {
                // Esc -- unfocus settings, move to chart display
                self.focus = ChartFocus::ChartDisplay;
                Ok(Some(Action::Render))
            }
        }
    }

    /// Handle key when chart display is focused.
    #[allow(clippy::unnecessary_wraps, clippy::unused_self)]
    fn handle_display_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        match key.code {
            KeyCode::Esc => {
                // Bubble up to QueryPane
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    /// Toggle settings visibility (Ctrl+C). Returns true if handled.
    pub fn toggle_settings(&mut self) {
        if self.settings_visible {
            self.settings_visible = false;
            self.focus = ChartFocus::ChartDisplay;
        } else {
            self.settings_visible = true;
            self.focus = ChartFocus::Settings;
        }
    }

    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        config: &mut ChartConfig,
    ) -> Result<Option<Action>> {
        // If a popup is open in settings, it captures all input
        if self.settings.popup_open.is_some() {
            return self.handle_settings_key(key, config);
        }

        match self.focus {
            ChartFocus::Settings => self.handle_settings_key(key, config),
            ChartFocus::ChartDisplay => self.handle_display_key(key),
        }
    }

    /// Render the chart content area (settings panel + chart display).
    /// The popup overlay is rendered separately via `render_popup()`.
    /// Config is passed from `QueryPane` so settings can display current values.
    pub fn render_content(&mut self, frame: &mut Frame, area: Rect, config: &ChartConfig) {
        if area.height < 2 || area.width < 4 {
            return;
        }

        // Reserve 1 line for hint bar at the bottom
        let hint_height = 1u16;
        let main_area;
        let hint_area;
        if area.height > hint_height + 2 {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(hint_height)])
                .split(area);
            main_area = chunks[0];
            hint_area = Some(chunks[1]);
        } else {
            main_area = area;
            hint_area = None;
        }

        if self.settings_visible && main_area.width >= SETTINGS_WIDTH + 10 {
            // Split: settings (fixed) + chart (remaining)
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(SETTINGS_WIDTH), Constraint::Min(0)])
                .split(main_area);

            self.render_settings(frame, split[0], config);
            self.render_chart_display(frame, split[1]);
        } else {
            // Chart only (settings hidden or too narrow)
            self.render_chart_display(frame, main_area);
        }

        // Hint bar
        if let Some(hint_area) = hint_area {
            self.render_hint_bar(frame, hint_area);
        }
    }

    /// Render the popup overlay on top of everything (called after all other rendering).
    pub fn render_popup(&self, frame: &mut Frame, area: Rect, config: &ChartConfig) {
        self.settings.render_popup(frame, area, config);
    }

    fn render_settings(&self, frame: &mut Frame, area: Rect, config: &ChartConfig) {
        let has_data = self.result.is_some();
        let focused = self.focus == ChartFocus::Settings;
        self.settings.render(frame, area, config, focused, has_data);
    }

    fn render_chart_display(&self, frame: &mut Frame, area: Rect) {
        if area.height < 2 || area.width < 4 {
            return;
        }

        match &self.prepared {
            Some(Ok(prepared)) => {
                render::render_chart(frame, area, prepared);
            }
            Some(Err(err)) => {
                render::render_error(frame, area, err);
            }
            None => {
                if self.result.is_none() {
                    render::render_empty(
                        frame,
                        area,
                        "No chart available -- need a label column and a numeric column",
                    );
                } else {
                    render::render_empty(frame, area, "Ctrl+C: Configure chart settings");
                }
            }
        }
    }

    fn render_hint_bar(&self, frame: &mut Frame, area: Rect) {
        use crate::theme;
        use ratatui::style::Style;
        use ratatui::text::{Line, Span};
        use ratatui::widgets::Paragraph;

        let accent = Style::default().fg(theme::accent_dim());
        let dim = Style::default().fg(theme::fg_dim());
        let hint_line = match self.focus {
            ChartFocus::Settings => Line::from(vec![
                Span::styled(" Esc", accent),
                Span::styled(": Back to chart  ", dim),
                Span::styled("Ctrl+C", accent),
                Span::styled(": Hide settings", dim),
            ]),
            ChartFocus::ChartDisplay => Line::from(vec![
                Span::styled(" T", accent),
                Span::styled(": Table View  ", dim),
                Span::styled("D", accent),
                Span::styled(": Delete  ", dim),
                Span::styled("Ctrl+N", accent),
                Span::styled(": New  ", dim),
                Span::styled("Ctrl+E", accent),
                Span::styled(": Edit Title  ", dim),
                Span::styled("Ctrl+C", accent),
                Span::styled(": Configure", dim),
            ]),
        };

        frame.render_widget(
            Paragraph::new(hint_line)
                .style(Style::default().bg(theme::bg_surface())),
            area,
        );
    }
}
