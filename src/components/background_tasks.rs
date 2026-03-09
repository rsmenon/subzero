#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_lossless, clippy::too_many_lines)]

use std::cell::Cell;

use color_eyre::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell as TableCell, Clear, Row, Table as TableWidget};

use crate::action::Action;
use crate::theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    Running,
    Complete,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskStatus::Pending => write!(f, "Pending"),
            TaskStatus::Running => write!(f, "Running"),
            TaskStatus::Complete => write!(f, "Complete"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BackgroundTask {
    pub id: usize,
    pub status: TaskStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub source: String,
    pub name: String,
}

#[derive(Debug)]
pub struct BackgroundTaskManager {
    tasks: Vec<BackgroundTask>,
    next_id: usize,
}

impl BackgroundTaskManager {
    pub fn new() -> Self {
        Self {
            tasks: Vec::new(),
            next_id: 1,
        }
    }

    pub fn create_task(&mut self, source: &str, name: &str) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.tasks.push(BackgroundTask {
            id,
            status: TaskStatus::Pending,
            created_at: chrono::Utc::now(),
            completed_at: None,
            source: source.to_string(),
            name: name.to_string(),
        });
        id
    }

    pub fn start_task(&mut self, id: usize) {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) {
            task.status = TaskStatus::Running;
        }
    }

    pub fn complete_task(&mut self, id: usize) {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) {
            task.status = TaskStatus::Complete;
            task.completed_at = Some(chrono::Utc::now());
        }
        // Prune: keep at most 200 completed tasks (remove oldest completed first)
        let completed_count = self
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Complete)
            .count();
        if completed_count > 200 {
            let to_remove = completed_count - 200;
            let mut removed = 0;
            self.tasks.retain(|t| {
                if t.status == TaskStatus::Complete && removed < to_remove {
                    removed += 1;
                    false
                } else {
                    true
                }
            });
        }
    }

    pub fn running_count(&self, source: &str) -> usize {
        self.tasks
            .iter()
            .filter(|t| t.source == source && t.status == TaskStatus::Running)
            .count()
    }

    pub fn pending_or_running_count(&self, source: &str) -> usize {
        self.tasks
            .iter()
            .filter(|t| {
                t.source == source
                    && (t.status == TaskStatus::Pending || t.status == TaskStatus::Running)
            })
            .count()
    }

    /// Find a running (or pending) task by source and name, return its ID
    pub fn find_running_task(&self, source: &str, name: &str) -> Option<usize> {
        self.tasks
            .iter()
            .rev() // search newest first
            .find(|t| {
                t.source == source
                    && t.name == name
                    && (t.status == TaskStatus::Running || t.status == TaskStatus::Pending)
            })
            .map(|t| t.id)
    }

    pub fn tasks(&self) -> &[BackgroundTask] {
        &self.tasks
    }
}

pub struct BackgroundTasksPane {
    pub visible: bool,
    selected: Cell<usize>,
    scroll_offset: usize,
}

impl BackgroundTasksPane {
    pub fn new() -> Self {
        Self {
            visible: false,
            selected: Cell::new(0),
            scroll_offset: 0,
        }
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    #[allow(clippy::unnecessary_wraps)]
    pub fn handle_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        if !self.visible {
            return Ok(None);
        }

        match key.code {
            KeyCode::Esc => {
                self.visible = false;
                Ok(Some(Action::Render))
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected.set(self.selected.get().saturating_add(1));
                self.adjust_scroll();
                Ok(Some(Action::Render))
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected.set(self.selected.get().saturating_sub(1));
                self.adjust_scroll();
                Ok(Some(Action::Render))
            }
            _ => Ok(Some(Action::None)),
        }
    }

    fn adjust_scroll(&mut self) {
        let sel = self.selected.get();
        if sel < self.scroll_offset {
            self.scroll_offset = sel;
        } else if sel >= self.scroll_offset + 18 {
            self.scroll_offset = sel.saturating_sub(17);
        }
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_lossless)]
    pub fn render(&self, frame: &mut Frame, area: Rect, manager: &BackgroundTaskManager) {
        if !self.visible {
            return;
        }

        let dialog_width = (area.width as f32 * 0.8).min(100.0) as u16;
        let dialog_height = (area.height as f32 * 0.7).min(30.0) as u16;

        let vertical = Layout::vertical([Constraint::Length(dialog_height)])
            .flex(Flex::Center)
            .split(area);
        let horizontal = Layout::horizontal([Constraint::Length(dialog_width)])
            .flex(Flex::Center)
            .split(vertical[0]);
        let dialog_area = horizontal[0];

        frame.render_widget(Clear, dialog_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::dialog_border()))
            .title(" Background Tasks ")
            .title_style(Style::default().fg(theme::accent_dim()))
            .style(Style::default().bg(theme::dialog_bg()).fg(theme::fg()));

        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);

        if inner.height < 3 || inner.width < 4 {
            return;
        }

        // Split inner: content + hint bar at bottom
        let inner_chunks = Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(inner);
        let content_area = inner_chunks[0];
        let hint_area = inner_chunks[1];

        // Render hint bar
        let accent = Style::default().fg(theme::accent_dim());
        let dim = Style::default().fg(theme::fg_dim());
        let hint_line = ratatui::text::Line::from(vec![
            ratatui::text::Span::styled(" j/k", accent),
            ratatui::text::Span::styled(": Navigate  ", dim),
            ratatui::text::Span::styled("Esc", accent),
            ratatui::text::Span::styled(": Close", dim),
        ]);
        let hint_paragraph = ratatui::widgets::Paragraph::new(hint_line)
            .style(Style::default().bg(theme::dialog_bg()));
        frame.render_widget(hint_paragraph, hint_area);

        let tasks = manager.tasks();

        // Clamp selected and store the clamped value
        let max_sel = tasks.len().saturating_sub(1);
        let selected = self.selected.get().min(max_sel);
        self.selected.set(selected);

        if tasks.is_empty() {
            let paragraph = ratatui::widgets::Paragraph::new("No background tasks")
                .style(Style::default().fg(theme::fg_dim()).bg(theme::dialog_bg()));
            frame.render_widget(paragraph, content_area);
            return;
        }

        // Header with per-cell bg so gap reveals dark BG_SURFACE
        let header_labels = ["STATUS", "CREATED", "SOURCE", "TASK NAME", "COMPLETED"];
        let header_cells: Vec<TableCell> = header_labels
            .iter()
            .map(|&label| {
                TableCell::from(label).style(
                    Style::default()
                        .fg(theme::accent_dim())
                        .bg(theme::dialog_bg())
                        .add_modifier(Modifier::BOLD),
                )
            })
            .collect();
        let header = Row::new(header_cells);

        let visible_count = content_area.height.saturating_sub(2) as usize;
        let rows: Vec<Row> = tasks
            .iter()
            .rev() // newest first
            .skip(self.scroll_offset)
            .take(visible_count)
            .enumerate()
            .map(|(i, task)| {
                let (status_text, status_color) = match task.status {
                    TaskStatus::Pending => ("Pending", theme::yellow()),
                    TaskStatus::Running => ("Running", theme::accent()),
                    TaskStatus::Complete => ("Complete", theme::green()),
                };

                let created = task.created_at.format("%H:%M:%S").to_string();
                let completed = task
                    .completed_at.map_or_else(|| "-".to_string(), |t| t.format("%H:%M:%S").to_string());

                let is_selected = (i + self.scroll_offset) == selected;
                let bg = if is_selected {
                    theme::bg_highlight()
                } else {
                    theme::dialog_bg()
                };

                let values: Vec<(String, ratatui::style::Color)> = vec![
                    (status_text.to_string(), status_color),
                    (created, theme::fg_dim()),
                    (task.source.clone(), theme::fg()),
                    (task.name.clone(), theme::fg_bright()),
                    (completed, theme::fg_dim()),
                ];

                let cells: Vec<TableCell> = values
                    .into_iter()
                    .map(|(text, fg)| {
                        let fg = if is_selected && fg == theme::fg() {
                            theme::fg_bright()
                        } else {
                            fg
                        };
                        TableCell::from(text).style(Style::default().fg(fg).bg(bg))
                    })
                    .collect();

                Row::new(cells)
            })
            .collect();

        let widths = [
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(18),
            Constraint::Min(20),
            Constraint::Length(10),
        ];

        let table = TableWidget::new(rows, widths)
            .header(header)
            .column_spacing(1)
            .style(Style::default().bg(theme::bg_surface()).fg(theme::fg()));

        frame.render_widget(table, content_area);
    }
}
