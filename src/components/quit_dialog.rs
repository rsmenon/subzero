use color_eyre::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::action::Action;
use crate::components::Component;
use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuitSelection {
    Yes,
    No,
}

pub struct QuitDialog {
    pub visible: bool,
    selection: QuitSelection,
}

impl QuitDialog {
    pub fn new() -> Self {
        Self {
            visible: false,
            selection: QuitSelection::Yes,
        }
    }

    pub fn show(&mut self) {
        self.visible = true;
        self.selection = QuitSelection::Yes;
    }

    fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
        let vertical = Layout::vertical([Constraint::Length(height)])
            .flex(Flex::Center)
            .split(area);
        let horizontal = Layout::horizontal([Constraint::Length(width)])
            .flex(Flex::Center)
            .split(vertical[0]);
        horizontal[0]
    }

    fn button_span(label: &str, focused: bool) -> Span<'_> {
        if focused {
            Span::styled(
                label,
                Style::default()
                    .fg(theme::fg_bright())
                    .bg(theme::accent_dim())
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(
                label,
                Style::default().fg(theme::fg()).bg(theme::bg_highlight()),
            )
        }
    }
}

impl Component for QuitDialog {
    fn update(&mut self, _action: Action) -> Result<Option<Action>> {
        Ok(None)
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        if !self.visible {
            return;
        }

        let dialog_area = Self::centered_rect(30, 5, area);

        // Clear the area behind the dialog
        frame.render_widget(Clear, dialog_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::dialog_border()))
            .style(Style::default().bg(theme::dialog_bg()).fg(theme::fg()))
            .title(" Quit ")
            .title_style(Style::default().fg(theme::accent_dim()));

        let text = Line::from(vec![
            Span::styled("Quit? ", Style::default().fg(theme::fg())),
            Self::button_span(" Yes ", self.selection == QuitSelection::Yes),
            Span::styled(" ", Style::default()),
            Self::button_span(" No ", self.selection == QuitSelection::No),
        ]);

        let paragraph = Paragraph::new(text)
            .block(block)
            .alignment(ratatui::layout::Alignment::Center);

        frame.render_widget(paragraph, dialog_area);
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        if !self.visible {
            return Ok(None);
        }

        // Capture ALL key events while visible
        match key.code {
            KeyCode::Left | KeyCode::Char('h') => {
                self.selection = QuitSelection::Yes;
                Ok(Some(Action::Render))
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.selection = QuitSelection::No;
                Ok(Some(Action::Render))
            }
            KeyCode::Enter => {
                self.visible = false;
                match self.selection {
                    QuitSelection::Yes => Ok(Some(Action::Quit)),
                    QuitSelection::No => Ok(Some(Action::HideQuitDialog)),
                }
            }
            KeyCode::Char('y' | 'Y') => {
                self.visible = false;
                Ok(Some(Action::Quit))
            }
            KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                self.visible = false;
                Ok(Some(Action::HideQuitDialog))
            }
            _ => Ok(Some(Action::None)), // Consume event, do nothing
        }
    }
}
