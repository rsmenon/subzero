pub mod background_tasks;
pub mod explore;
pub mod query;
pub mod quit_dialog;
pub mod row_detail;
pub mod saved_queries;
pub mod settings;
pub mod top_bar;

use color_eyre::Result;
use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::prelude::Rect;

use crate::action::Action;

pub trait Component {
    fn update(&mut self, action: Action) -> Result<Option<Action>>;
    fn render(&mut self, frame: &mut Frame, area: Rect);
    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<Action>>;
}

/// Format a JSON value for display in tables and detail views.
pub fn format_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "NULL".to_string(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}
