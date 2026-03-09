use color_eyre::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::prelude::Direction;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::action::Action;
use crate::app::Mode;
use crate::components::Component;
use crate::theme;

pub struct TopBar {
    pub focused: bool,
}

impl TopBar {
    pub fn new() -> Self {
        Self { focused: true }
    }

    fn render_tab(&self, label: &str, hint: char, is_active: bool) -> Vec<Span<'static>> {
        let (bg, fg, mods) = if is_active {
            (theme::tab_active_bg(), theme::fg_bright(), Modifier::BOLD)
        } else {
            (theme::tab_inactive_bg(), theme::fg_dim(), Modifier::empty())
        };

        if self.focused {
            vec![
                Span::styled(
                    format!(" [{}]", hint.to_uppercase()),
                    Style::default().fg(theme::accent_dim()).bg(bg),
                ),
                Span::styled(
                    format!("{} ", &label.chars().skip(1).collect::<String>()),
                    Style::default().fg(fg).bg(bg).add_modifier(mods),
                ),
            ]
        } else {
            vec![Span::styled(
                format!(" {label} "),
                Style::default().fg(fg).bg(bg).add_modifier(mods),
            )]
        }
    }
}

impl Component for TopBar {
    fn update(&mut self, _action: Action) -> Result<Option<Action>> {
        Ok(None)
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Fallback: App calls render_with_state() instead
        let bg = Style::default().bg(theme::bg_surface());
        let block = Block::default().style(bg);
        frame.render_widget(block, area);
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        if !self.focused {
            return Ok(None);
        }
        match key.code {
            KeyCode::Char('e' | 'E') => Ok(Some(Action::SwitchMode(Mode::Explore))),
            KeyCode::Char('q' | 'Q') => Ok(Some(Action::SwitchMode(Mode::Query))),
            KeyCode::Char('s' | 'S') => Ok(Some(Action::SwitchMode(Mode::Settings))),
            _ => Ok(None),
        }
    }
}

impl TopBar {
    pub fn render_with_state(
        &self,
        frame: &mut Frame,
        area: Rect,
        active_mode: Mode,
        connected: Option<bool>,
    ) {
        let bg_style = Style::default().bg(theme::bg_surface());

        // Always show border; use BORDER_FOCUS when focused, BORDER when not
        let border_color = if self.focused {
            theme::border_focus()
        } else {
            theme::border()
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(bg_style);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Split area into tabs (left) and status (right)
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(40), Constraint::Length(20)])
            .split(inner);

        // Render tabs
        let modes = [
            ("Explore", 'E', Mode::Explore),
            ("Query", 'Q', Mode::Query),
            ("Settings", 'S', Mode::Settings),
        ];

        let mut spans: Vec<Span<'static>> = Vec::new();

        // Logo: ❄❯SUBZERO❮❄ (with left padding)
        let logo_bg = Style::default().bg(theme::bg_surface());
        spans.push(Span::styled(" ", logo_bg));
        spans.push(Span::styled("❄", logo_bg.fg(theme::accent_dim())));
        spans.push(Span::styled(
            "❯",
            logo_bg.fg(theme::fg_bright()).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            "SUBZERO",
            logo_bg.fg(theme::accent_dim()).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            "❮",
            logo_bg.fg(theme::fg_bright()).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled("❄", logo_bg.fg(theme::accent_dim())));
        spans.push(Span::styled(" ", logo_bg));
        spans.push(Span::styled(
            "│",
            Style::default().fg(theme::border()).bg(theme::bg_surface()),
        ));
        spans.push(Span::styled(" ", logo_bg));

        for (label, hint, mode) in &modes {
            let is_active = std::mem::discriminant(&active_mode) == std::mem::discriminant(mode);
            spans.extend(self.render_tab(label, *hint, is_active));
            spans.push(Span::styled(" ", Style::default().bg(theme::bg_surface())));
        }

        let tabs_line = Line::from(spans);
        let tabs = Paragraph::new(tabs_line).style(bg_style);
        frame.render_widget(tabs, chunks[0]);

        // Render connection status
        let (status_text, status_color) = match connected {
            Some(true) => ("\u{25cf} Connected", theme::green()),
            Some(false) => ("\u{25cf} Disconnected", theme::red()),
            None => ("\u{25cf} Checking...", theme::yellow()),
        };

        let status = Paragraph::new(Line::from(Span::styled(
            status_text,
            Style::default().fg(status_color).bg(theme::bg_surface()),
        )))
        .alignment(Alignment::Right)
        .style(bg_style);
        frame.render_widget(status, chunks[1]);
    }
}
