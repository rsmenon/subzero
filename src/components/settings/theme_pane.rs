#![allow(clippy::cast_possible_truncation, clippy::too_many_lines)]

use std::time::Instant;

use color_eyre::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::action::Action;
use crate::theme;
use crate::theme::ThemeConfig;

/// How long status messages remain visible (seconds)
const STATUS_TIMEOUT_SECS: u64 = 3;

/// Which area within the theme pane has focus
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThemeFocusArea {
    ColorList,
    Buttons,
}

/// Which button is focused in the button area
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThemeButton {
    Save,
    ResetDefaults,
}

/// Ordered list of all theme fields
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThemeField {
    Bg,
    BgSurface,
    BgHighlight,
    Fg,
    FgDim,
    FgMid,
    FgBright,
    Accent,
    AccentDim,
    TabActiveBg,
    TabInactiveBg,
    Border,
    BorderFocus,
    Green,
    Red,
    Yellow,
    ChartPurple,
    ChartMint,
    DialogBg,
    DialogBorder,
}

impl ThemeField {
    fn all() -> &'static [ThemeField] {
        &[
            ThemeField::Bg,
            ThemeField::BgSurface,
            ThemeField::BgHighlight,
            ThemeField::Fg,
            ThemeField::FgDim,
            ThemeField::FgMid,
            ThemeField::FgBright,
            ThemeField::Accent,
            ThemeField::AccentDim,
            ThemeField::TabActiveBg,
            ThemeField::TabInactiveBg,
            ThemeField::Border,
            ThemeField::BorderFocus,
            ThemeField::Green,
            ThemeField::Red,
            ThemeField::Yellow,
            ThemeField::ChartPurple,
            ThemeField::ChartMint,
            ThemeField::DialogBg,
            ThemeField::DialogBorder,
        ]
    }

    fn label(self) -> &'static str {
        match self {
            ThemeField::Bg => "Background",
            ThemeField::BgSurface => "Surface",
            ThemeField::BgHighlight => "Highlight",
            ThemeField::Fg => "Foreground",
            ThemeField::FgDim => "Fg Dim",
            ThemeField::FgMid => "Fg Mid",
            ThemeField::FgBright => "Fg Bright",
            ThemeField::Accent => "Accent",
            ThemeField::AccentDim => "Accent Dim",
            ThemeField::TabActiveBg => "Tab Active",
            ThemeField::TabInactiveBg => "Tab Inactive",
            ThemeField::Border => "Border",
            ThemeField::BorderFocus => "Border Focus",
            ThemeField::Green => "Green",
            ThemeField::Red => "Red",
            ThemeField::Yellow => "Yellow",
            ThemeField::ChartPurple => "Chart Purple",
            ThemeField::ChartMint => "Chart Mint",
            ThemeField::DialogBg => "Dialog Bg",
            ThemeField::DialogBorder => "Dialog Border",
        }
    }

    fn get(self, config: &ThemeConfig) -> &str {
        match self {
            ThemeField::Bg => &config.bg,
            ThemeField::BgSurface => &config.bg_surface,
            ThemeField::BgHighlight => &config.bg_highlight,
            ThemeField::Fg => &config.fg,
            ThemeField::FgDim => &config.fg_dim,
            ThemeField::FgMid => &config.fg_mid,
            ThemeField::FgBright => &config.fg_bright,
            ThemeField::Accent => &config.accent,
            ThemeField::AccentDim => &config.accent_dim,
            ThemeField::TabActiveBg => &config.tab_active_bg,
            ThemeField::TabInactiveBg => &config.tab_inactive_bg,
            ThemeField::Border => &config.border,
            ThemeField::BorderFocus => &config.border_focus,
            ThemeField::Green => &config.green,
            ThemeField::Red => &config.red,
            ThemeField::Yellow => &config.yellow,
            ThemeField::ChartPurple => &config.chart_purple,
            ThemeField::ChartMint => &config.chart_mint,
            ThemeField::DialogBg => &config.dialog_bg,
            ThemeField::DialogBorder => &config.dialog_border,
        }
    }

    fn set(self, config: &mut ThemeConfig, value: String) {
        match self {
            ThemeField::Bg => config.bg = value,
            ThemeField::BgSurface => config.bg_surface = value,
            ThemeField::BgHighlight => config.bg_highlight = value,
            ThemeField::Fg => config.fg = value,
            ThemeField::FgDim => config.fg_dim = value,
            ThemeField::FgMid => config.fg_mid = value,
            ThemeField::FgBright => config.fg_bright = value,
            ThemeField::Accent => config.accent = value,
            ThemeField::AccentDim => config.accent_dim = value,
            ThemeField::TabActiveBg => config.tab_active_bg = value,
            ThemeField::TabInactiveBg => config.tab_inactive_bg = value,
            ThemeField::Border => config.border = value,
            ThemeField::BorderFocus => config.border_focus = value,
            ThemeField::Green => config.green = value,
            ThemeField::Red => config.red = value,
            ThemeField::Yellow => config.yellow = value,
            ThemeField::ChartPurple => config.chart_purple = value,
            ThemeField::ChartMint => config.chart_mint = value,
            ThemeField::DialogBg => config.dialog_bg = value,
            ThemeField::DialogBorder => config.dialog_border = value,
        }
    }
}

pub struct ThemeEditorPane {
    /// Which color entry is selected
    selected_index: usize,
    /// Scroll offset for the list
    scroll_offset: usize,
    /// The working copy of the theme config (editable, not yet saved)
    working_config: ThemeConfig,
    /// Whether we are in hex edit mode for the selected entry
    editing: bool,
    /// The hex input buffer while editing
    edit_buffer: String,
    /// Error message for invalid hex (shown inline)
    edit_error: Option<String>,
    /// Which button is focused (when in button area)
    button_focus: ThemeButton,
    /// Whether focus is on the list or the buttons
    focus_area: ThemeFocusArea,
    /// Status message with timestamp
    status: Option<(String, Instant)>,
}

impl ThemeEditorPane {
    pub fn new() -> Self {
        Self {
            selected_index: 0,
            scroll_offset: 0,
            working_config: ThemeConfig::default(),
            editing: false,
            edit_buffer: String::new(),
            edit_error: None,
            button_focus: ThemeButton::Save,
            focus_area: ThemeFocusArea::ColorList,
            status: None,
        }
    }

    /// Load the current applied theme config into the working copy
    pub fn load_from_config(&mut self, config: ThemeConfig) {
        self.working_config = config;
    }

    /// Reload from the currently applied theme (after save/reset)
    pub fn reload_from_applied(&mut self) {
        self.working_config = theme::current_config();
        self.editing = false;
        self.edit_buffer.clear();
        self.edit_error = None;
    }

    /// Set a status message
    pub fn set_status(&mut self, msg: &str) {
        self.status = Some((msg.to_string(), Instant::now()));
    }

    /// Get a live status message, returning None if it has expired
    fn active_status(&self) -> Option<&str> {
        if let Some((msg, ts)) = &self.status
            && ts.elapsed().as_secs() < STATUS_TIMEOUT_SECS {
                return Some(msg.as_str());
            }
        None
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        // If editing a hex value, handle editing keys
        if self.editing {
            return self.handle_edit_key(key);
        }

        // Esc: bubble up (app.rs handles focus)
        if key.code == KeyCode::Esc {
            return Ok(None);
        }

        match self.focus_area {
            ThemeFocusArea::ColorList => self.handle_list_key(key),
            ThemeFocusArea::Buttons => self.handle_button_key(key),
        }
    }

    #[allow(clippy::unnecessary_wraps)]
    fn handle_list_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        let field_count = ThemeField::all().len();

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if self.selected_index + 1 < field_count {
                    self.selected_index += 1;
                    self.edit_error = None;
                } else {
                    // Move to buttons
                    self.focus_area = ThemeFocusArea::Buttons;
                    self.edit_error = None;
                }
                Ok(Some(Action::Render))
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.selected_index > 0 {
                    self.selected_index -= 1;
                    self.edit_error = None;
                }
                Ok(Some(Action::Render))
            }
            KeyCode::Enter => {
                // Start editing the selected field
                let field = ThemeField::all()[self.selected_index];
                self.edit_buffer = field.get(&self.working_config).to_string();
                self.editing = true;
                self.edit_error = None;
                Ok(Some(Action::Render))
            }
            _ => Ok(None),
        }
    }

    #[allow(clippy::unnecessary_wraps)]
    fn handle_button_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        match key.code {
            KeyCode::Char('k') | KeyCode::Up => {
                // Move back to color list (last entry)
                self.focus_area = ThemeFocusArea::ColorList;
                self.selected_index = ThemeField::all().len() - 1;
                Ok(Some(Action::Render))
            }
            KeyCode::Char('h') | KeyCode::Left => {
                self.button_focus = ThemeButton::Save;
                Ok(Some(Action::Render))
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.button_focus = ThemeButton::ResetDefaults;
                Ok(Some(Action::Render))
            }
            KeyCode::Enter | KeyCode::Char(' ') => match self.button_focus {
                ThemeButton::Save => {
                    // Validate all hex values before saving
                    for field in ThemeField::all() {
                        let hex = field.get(&self.working_config);
                        if theme::parse_hex(hex).is_none() {
                            self.set_status(&format!(
                                "Invalid hex for {}: {}",
                                field.label(),
                                hex
                            ));
                            return Ok(Some(Action::Render));
                        }
                    }
                    Ok(Some(Action::SaveTheme(Box::new(
                        self.working_config.clone(),
                    ))))
                }
                ThemeButton::ResetDefaults => {
                    self.working_config = ThemeConfig::default();
                    Ok(Some(Action::ResetTheme))
                }
            },
            _ => Ok(None),
        }
    }

    #[allow(clippy::unnecessary_wraps)]
    fn handle_edit_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        match key.code {
            KeyCode::Esc => {
                // Cancel editing, revert
                self.editing = false;
                self.edit_buffer.clear();
                self.edit_error = None;
                Ok(Some(Action::Render))
            }
            KeyCode::Enter => {
                // Validate and confirm
                let normalized = Self::normalize_hex(&self.edit_buffer.clone());
                if theme::parse_hex(&normalized).is_some() {
                    let field = ThemeField::all()[self.selected_index];
                    field.set(&mut self.working_config, normalized);
                    self.editing = false;
                    self.edit_buffer.clear();
                    self.edit_error = None;
                } else {
                    self.edit_error = Some("Invalid hex color".to_string());
                    self.editing = false;
                    self.edit_buffer.clear();
                }
                Ok(Some(Action::Render))
            }
            KeyCode::Backspace => {
                self.edit_buffer.pop();
                self.edit_error = None;
                Ok(Some(Action::Render))
            }
            KeyCode::Char(c) => {
                if (c.is_ascii_hexdigit() || c == '#')
                    && self.edit_buffer.chars().count() < 7
                {
                    self.edit_buffer.push(c);
                    self.edit_error = None;
                }
                Ok(Some(Action::Render))
            }
            _ => Ok(None),
        }
    }

    /// Normalize a hex input: ensure it starts with '#' and is lowercase
    fn normalize_hex(input: &str) -> String {
        let trimmed = input.trim();
        
        if trimmed.starts_with('#') {
            trimmed.to_lowercase()
        } else {
            format!("#{}", trimmed.to_lowercase())
        }
    }

    #[allow(clippy::unnecessary_wraps, clippy::needless_pass_by_value)]
    pub fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::ThemeSaved => {
                self.reload_from_applied();
                self.set_status("Theme saved and applied");
                Ok(Some(Action::Render))
            }
            _ => Ok(None),
        }
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let border_color = if focused {
            theme::border_focus()
        } else {
            theme::border()
        };

        let block = Block::default()
            .title(" Theme ")
            .title_style(Style::default().fg(theme::accent_dim()))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(theme::bg_surface()).fg(theme::fg()));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.height < 6 || inner.width < 30 {
            let msg = Paragraph::new("Too small")
                .style(Style::default().fg(theme::fg_dim()).bg(theme::bg_surface()));
            frame.render_widget(msg, inner);
            return;
        }

        let content = inner.inner(Margin::new(1, 0));

        // Layout: header + separator + color list + spacer + buttons + hint
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Header
                Constraint::Length(1), // Separator
                Constraint::Min(1),    // Color list
                Constraint::Length(1), // Spacer
                Constraint::Length(1), // Buttons
                Constraint::Length(1), // Hint bar
            ])
            .split(content);

        // Header row
        Self::render_header(frame, rows[0]);

        // Separator
        let sep_text = "\u{2500}".repeat(rows[1].width as usize);
        frame.render_widget(
            Paragraph::new(sep_text)
                .style(Style::default().fg(theme::border()).bg(theme::bg_surface())),
            rows[1],
        );

        // Color list (scrollable)
        self.render_color_list(frame, rows[2], focused);

        // Buttons
        self.render_buttons(frame, rows[4], focused);

        // Hint bar + status
        if focused {
            self.render_hint_bar(frame, rows[5]);
        }
    }

    fn render_header(frame: &mut Frame, area: Rect) {
        let label_w = 16;
        let hex_w = 9;

        let header = Line::from(vec![
            Span::styled(
                format!("{:<width$}", "VARIABLE", width = label_w),
                Style::default()
                    .fg(theme::fg_dim())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<width$}", "HEX", width = hex_w),
                Style::default()
                    .fg(theme::fg_dim())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "SWATCH",
                Style::default()
                    .fg(theme::fg_dim())
                    .add_modifier(Modifier::BOLD),
            ),
        ]);

        frame.render_widget(
            Paragraph::new(header).style(Style::default().bg(theme::bg_surface())),
            area,
        );
    }

    fn render_color_list(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        let fields = ThemeField::all();
        let visible_count = area.height as usize;

        // Update scroll offset to keep selection visible
        if self.focus_area == ThemeFocusArea::ColorList {
            if self.selected_index >= self.scroll_offset + visible_count {
                self.scroll_offset = self.selected_index + 1 - visible_count;
            }
            if self.selected_index < self.scroll_offset {
                self.scroll_offset = self.selected_index;
            }
        }
        let scroll = self.scroll_offset;

        let label_w = 16;
        let hex_w = 9;

        for (i, field) in fields
            .iter()
            .enumerate()
            .skip(scroll)
            .take(visible_count)
        {
            let y = area.y + (i - scroll) as u16;
            if y >= area.y + area.height {
                break;
            }

            let is_selected =
                focused && self.focus_area == ThemeFocusArea::ColorList && i == self.selected_index;
            let is_editing = is_selected && self.editing;
            let has_error = is_selected && self.edit_error.is_some();

            let hex_value = field.get(&self.working_config);

            // Label
            let label_style = if is_selected {
                Style::default().fg(theme::accent())
            } else {
                Style::default().fg(theme::fg_dim())
            };

            // Hex value
            let (hex_display, hex_style) = if is_editing {
                let cursor = "\u{2588}";
                let display = format!("{}{}", self.edit_buffer, cursor);
                let style = Style::default()
                    .fg(theme::fg_bright())
                    .bg(theme::bg_highlight());
                (display, style)
            } else if has_error {
                (
                    hex_value.to_string(),
                    Style::default().fg(theme::red()),
                )
            } else {
                (
                    hex_value.to_string(),
                    Style::default().fg(theme::fg()),
                )
            };

            // Swatch
            let swatch_color = theme::parse_hex(if is_editing {
                &self.edit_buffer
            } else {
                hex_value
            });

            let (swatch_text, swatch_style) = if let Some(color) = swatch_color {
                ("  ", Style::default().bg(color))
            } else {
                ("??", Style::default().fg(theme::red()))
            };

            let row_area = Rect::new(area.x, y, area.width, 1);

            let mut spans = vec![
                Span::styled(
                    format!("{:<width$}", field.label(), width = label_w),
                    label_style,
                ),
                Span::styled(format!("{hex_display:<hex_w$}"), hex_style),
                Span::styled(swatch_text, swatch_style),
            ];

            // Show error inline after swatch
            if has_error
                && let Some(ref err) = self.edit_error {
                    spans.push(Span::styled(
                        format!("  {err}"),
                        Style::default().fg(theme::red()),
                    ));
                }

            let line = Line::from(spans);
            let bg = if is_selected {
                theme::bg_highlight()
            } else {
                theme::bg_surface()
            };
            frame.render_widget(
                Paragraph::new(line).style(Style::default().bg(bg)),
                row_area,
            );
        }
    }

    fn render_buttons(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let btn_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(8),
                Constraint::Length(2),
                Constraint::Length(20),
                Constraint::Min(0),
            ])
            .split(area);

        let save_focused =
            focused && self.focus_area == ThemeFocusArea::Buttons && self.button_focus == ThemeButton::Save;
        let reset_focused = focused
            && self.focus_area == ThemeFocusArea::Buttons
            && self.button_focus == ThemeButton::ResetDefaults;

        frame.render_widget(Self::render_button("Save", save_focused), btn_layout[0]);
        frame.render_widget(
            Self::render_button("Reset to Defaults", reset_focused),
            btn_layout[2],
        );

        // Show status message after buttons if available
        if let Some(status) = self.active_status()
            && btn_layout[3].width > 2 {
                let status_span = Span::styled(
                    format!(" {status}"),
                    Style::default().fg(theme::green()),
                );
                frame.render_widget(
                    Paragraph::new(Line::from(status_span))
                        .style(Style::default().bg(theme::bg_surface())),
                    btn_layout[3],
                );
            }
    }

    fn render_button(label: &str, focused: bool) -> Paragraph<'_> {
        let style = if focused {
            Style::default()
                .fg(theme::fg_bright())
                .bg(theme::accent_dim())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(theme::fg())
                .bg(theme::bg_highlight())
        };
        Paragraph::new(format!(" {label} ")).style(style)
    }

    fn render_hint_bar(&self, frame: &mut Frame, area: Rect) {
        let dim = Style::default().fg(theme::fg_dim());
        let accent = Style::default().fg(theme::accent_dim());

        let hints = if self.editing {
            Line::from(vec![
                Span::styled(" Type", accent),
                Span::styled(": Hex  ", dim),
                Span::styled("Enter", accent),
                Span::styled(": Confirm  ", dim),
                Span::styled("Esc", accent),
                Span::styled(": Cancel", dim),
            ])
        } else {
            match self.focus_area {
                ThemeFocusArea::ColorList => Line::from(vec![
                    Span::styled(" j/k", accent),
                    Span::styled(": Navigate  ", dim),
                    Span::styled("Enter", accent),
                    Span::styled(": Edit  ", dim),
                    Span::styled("Shift+Tab", accent),
                    Span::styled(": Cache", dim),
                ]),
                ThemeFocusArea::Buttons => Line::from(vec![
                    Span::styled(" h/l", accent),
                    Span::styled(": Switch  ", dim),
                    Span::styled("Enter", accent),
                    Span::styled(": Activate  ", dim),
                    Span::styled("k", accent),
                    Span::styled(": Back to list", dim),
                ]),
            }
        };

        frame.render_widget(
            Paragraph::new(hints).style(Style::default().bg(theme::bg_surface())),
            area,
        );
    }
}
