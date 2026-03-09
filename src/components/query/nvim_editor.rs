#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::too_many_lines)]

use std::io::{Read, Write as IoWrite};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use color_eyre::Result;
use color_eyre::eyre::eyre;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::action::Action;
use crate::event::AppEvent;
use crate::theme;

use super::nvim_rpc::NvimRpcSync;

/// Status of the embedded nvim process
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NvimStatus {
    /// Not yet started
    NotStarted,
    /// Running normally
    Running,
    /// Process exited or crashed
    Exited(String),
}

pub struct NvimEditor {
    /// PTY write handle for sending bytes to nvim
    pty_writer: Option<Box<dyn IoWrite + Send>>,
    /// PTY master handle for resize
    pty_master: Option<Box<dyn portable_pty::MasterPty + Send>>,
    /// Child process handle (kept alive to avoid zombies; killed on shutdown)
    child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
    /// Parsed terminal screen (shared with reader task)
    parser: Arc<Mutex<vt100::Parser>>,
    /// Temp file path for the SQL buffer
    tmp_path: PathBuf,
    /// Socket path for RPC sidecar
    socket_path: PathBuf,
    /// Whether the editor pane is focused
    pub focused: bool,
    /// Last known size for resize detection
    last_size: (u16, u16),
    /// Current status
    status: NvimStatus,
    /// Event sender for communicating nvim lifecycle events
    action_tx: Option<mpsc::UnboundedSender<AppEvent>>,
    /// Path to the bundled init.lua
    init_lua_path: PathBuf,
    /// Sync-accessible RPC state (buffer mirror, cached mode). None until RPC connects.
    rpc_sync: Option<NvimRpcSync>,
    /// Whether the async RPC handle is available (for deciding `set_sql` path)
    rpc_connected: bool,
    /// Number of spawn attempts (for infinite respawn loop prevention)
    spawn_attempts: u32,
    /// Timestamp of the last spawn (for detecting rapid crash loops)
    last_spawn: Option<std::time::Instant>,
}

impl NvimEditor {
    pub fn new() -> Self {
        let tmp_dir = std::env::temp_dir();
        let pid = std::process::id();
        let tmp_path = tmp_dir.join(format!("sz_nvim_{pid}.sql"));
        let socket_path = tmp_dir.join(format!("sz_nvim_{pid}.sock"));

        // Find the bundled init.lua relative to the executable
        let init_lua_path = find_init_lua();

        Self {
            pty_writer: None,
            pty_master: None,
            child: None,
            parser: Arc::new(Mutex::new(vt100::Parser::new(24, 80, 0))),
            tmp_path,
            socket_path,
            focused: true,
            last_size: (0, 0),
            status: NvimStatus::NotStarted,
            action_tx: None,
            init_lua_path,
            rpc_sync: None,
            rpc_connected: false,
            spawn_attempts: 0,
            last_spawn: None,
        }
    }

    /// Set the action sender for lifecycle events
    pub fn set_action_tx(&mut self, tx: mpsc::UnboundedSender<AppEvent>) {
        self.action_tx = Some(tx);
    }

    /// Spawn the nvim process inside a PTY. Must be called after layout is known.
    pub fn spawn(&mut self, rows: u16, cols: u16) -> Result<()> {
        if self.status == NvimStatus::Running {
            return Ok(());
        }

        // Clean up stale socket if present
        let _ = std::fs::remove_file(&self.socket_path);

        // Create the temp file if it does not exist
        if !self.tmp_path.exists() {
            std::fs::write(&self.tmp_path, "")?;
        }

        // Resize parser to match
        {
            let mut parser = self.parser.lock().expect("vt100 Parser Mutex poisoned");
            *parser = vt100::Parser::new(rows, cols, 0);
        }
        self.last_size = (cols, rows);

        // Create PTY
        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| eyre!("{}", e))?;

        // Build nvim command
        let mut cmd = CommandBuilder::new("nvim");
        cmd.arg("--clean");
        cmd.arg("-u");
        cmd.arg(self.init_lua_path.to_str().unwrap_or(""));
        cmd.arg("--listen");
        cmd.arg(self.socket_path.to_str().unwrap_or(""));
        cmd.arg(self.tmp_path.to_str().unwrap_or(""));

        // Set TERM for proper color support
        cmd.env("TERM", "xterm-256color");

        // Pass catalog JSON path for the completion plugin
        let catalog_path =
            std::env::temp_dir().join(format!("sz-catalog-{}.json", std::process::id()));
        cmd.env("SZ_CATALOG_PATH", catalog_path.to_str().unwrap_or(""));

        // Track spawn attempts for crash loop prevention
        self.spawn_attempts += 1;
        self.last_spawn = Some(std::time::Instant::now());

        // Spawn child -- store handle to avoid zombie processes
        let child = pair.slave.spawn_command(cmd).map_err(|e| eyre!("{}", e))?;
        self.child = Some(child);

        // Get write handle
        let writer = pair.master.take_writer().map_err(|e| eyre!("{}", e))?;
        self.pty_writer = Some(writer);

        // Start reader task
        let parser_clone = Arc::clone(&self.parser);
        let action_tx = self.action_tx.clone();
        let mut reader = pair.master.try_clone_reader().map_err(|e| eyre!("{}", e))?;

        // Store master handle for resize
        self.pty_master = Some(pair.master);

        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        // EOF -- nvim exited
                        if let Some(ref tx) = action_tx {
                            let _ = tx.send(AppEvent::Action(Action::NvimExited(
                                "nvim process exited".to_string(),
                            )));
                        }
                        break;
                    }
                    Ok(n) => {
                        let mut parser = parser_clone.lock().expect("vt100 Parser Mutex poisoned");
                        parser.process(&buf[..n]);
                    }
                    Err(e) => {
                        error!("PTY reader error: {}", e);
                        if let Some(ref tx) = action_tx {
                            let _ = tx.send(AppEvent::Action(Action::NvimExited(format!(
                                "PTY read error: {e}"
                            ))));
                        }
                        break;
                    }
                }
            }
        });

        self.status = NvimStatus::Running;
        info!("nvim spawned with PTY ({}x{})", cols, rows);

        // Notify that nvim is ready
        if let Some(ref tx) = self.action_tx {
            let _ = tx.send(AppEvent::Action(Action::NvimReady));
        }

        Ok(())
    }

    /// Get SQL content from the buffer.
    /// Priority 1: live buffer mirror from RPC `buf_attach` (instant, no I/O).
    /// Priority 2: existing PTY + temp file approach (fallback).
    pub fn get_sql(&mut self) -> String {
        // Try RPC mirror first (instant memory read)
        if let Some(ref rpc_sync) = self.rpc_sync
            && let Some(sql) = rpc_sync.get_mirror()
        {
            return sql.trim().to_string();
        }
        // Fallback: PTY-based approach
        self.get_sql_via_pty()
    }

    /// Original PTY-based `get_sql`: write buffer to disk, sleep, read file.
    fn get_sql_via_pty(&mut self) -> String {
        // First, tell nvim to write the buffer
        if let Some(ref mut writer) = self.pty_writer {
            // Send Escape first to ensure we're in normal mode, then :w
            let _ = writer.write_all(b"\x1b");
            let _ = writer.write_all(b":w\r");
            let _ = writer.flush();
            // TODO: this blocks the TUI for the sleep duration; acceptable only as a fallback path
            // Best-effort delay: nvim needs time to flush :w to disk before we read.
            // This is a fallback path only used when RPC mirror is unavailable.
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // Read the temp file
        std::fs::read_to_string(&self.tmp_path)
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    /// Set SQL content in the editor.
    /// If RPC is connected, dispatches an async action to set via API.
    /// Otherwise falls back to the PTY file-write approach.
    pub fn set_sql(&mut self, sql: &str) {
        if self.rpc_connected {
            // Dispatch async set via RPC — the caller will handle this action
            if let Some(ref tx) = self.action_tx {
                let _ = tx.send(AppEvent::Action(Action::SetEditorContent(sql.to_string())));
            }
            return;
        }
        self.set_sql_via_pty(sql);
    }

    /// Original PTY-based `set_sql`: write file, tell nvim to reload.
    fn set_sql_via_pty(&mut self, sql: &str) {
        // Write to temp file
        let trimmed = sql.trim_end();
        let _ = std::fs::write(&self.tmp_path, trimmed);

        // Tell nvim to reload the file
        if let Some(ref mut writer) = self.pty_writer {
            let _ = writer.write_all(b"\x1b");
            let _ = writer.write_all(b":e!\r");
            let _ = writer.flush();
        }
    }

    /// Handle resize if the area changed
    fn maybe_resize(&mut self, area: Rect) {
        // Account for borders (1 on each side), hint bar (1 row), and top padding (1 row)
        let inner_cols = area.width.saturating_sub(2);
        let inner_rows = area.height.saturating_sub(4); // 2 for borders + 1 for hint bar + 1 for top padding

        if inner_cols == 0 || inner_rows == 0 {
            return;
        }

        let new_size = (inner_cols, inner_rows);
        if new_size != self.last_size && self.status == NvimStatus::Running {
            self.last_size = new_size;
            // Resize the parser
            {
                let mut parser = self.parser.lock().expect("vt100 Parser Mutex poisoned");
                parser.set_size(inner_rows, inner_cols);
            }
            // Resize the PTY so nvim gets SIGWINCH
            if let Some(ref master) = self.pty_master {
                let _ = master.resize(PtySize {
                    rows: inner_rows,
                    cols: inner_cols,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            }
        }
    }

    /// Forward a key event to nvim as raw bytes
    fn write_key(&mut self, key: KeyEvent) {
        if let Some(ref mut writer) = self.pty_writer {
            let bytes = key_to_bytes(key);
            let slice = bytes.as_slice();
            if !slice.is_empty() {
                let _ = writer.write_all(slice);
                let _ = writer.flush();
            }
        }
    }

    /// Graceful shutdown
    pub fn shutdown(&mut self) {
        // Drop RPC state
        self.rpc_sync = None;
        self.rpc_connected = false;

        if self.status == NvimStatus::Running {
            // Kill the child process first to prevent zombies
            if let Some(ref mut child) = self.child {
                let _ = child.kill();
            }
            if let Some(ref mut writer) = self.pty_writer {
                let _ = writer.write_all(b"\x1b");
                let _ = writer.write_all(b":qa!\r");
                let _ = writer.flush();
            }
            self.status = NvimStatus::Exited("shutdown".to_string());
        }
        // Drop the child handle so the OS can reap the process
        self.child = None;
        // Clean up temp files
        let _ = std::fs::remove_file(&self.tmp_path);
        let _ = std::fs::remove_file(&self.socket_path);
    }

    /// Check if nvim is currently in normal mode.
    /// Uses RPC cached mode when available, falls back to screen scraping.
    pub fn is_normal_mode(&self) -> bool {
        if self.status != NvimStatus::Running {
            return true;
        }

        // Try RPC cached mode first (more reliable)
        if let Some(ref rpc_sync) = self.rpc_sync
            && let Some(mode) = rpc_sync.get_cached_mode()
        {
            // "n" = normal, "no" = operator-pending (treat as normal for Esc purposes)
            // "i" = insert, "v"/"V"/"\x16" = visual, "R" = replace, "s"/"S" = select
            return mode == "n" || mode == "no" || mode.starts_with("nt");
        }

        // Fallback: screen scraping
        self.is_normal_mode_via_screen()
    }

    /// Original screen-scraping mode detection.
    fn is_normal_mode_via_screen(&self) -> bool {
        let parser = self.parser.lock().expect("vt100 Parser Mutex poisoned");
        let screen = parser.screen();
        let rows = screen.size().0;
        if rows == 0 {
            return true;
        }
        // Check the last row for mode indicators
        let last_row = rows - 1;
        let cols = screen.size().1;
        let mut line = String::new();
        for col in 0..cols {
            if let Some(cell) = screen.cell(last_row, col) {
                let contents = cell.contents();
                if contents.is_empty() {
                    line.push(' ');
                } else {
                    line.push_str(&contents);
                }
            }
        }
        let line_lower = line.to_lowercase();
        // If the status line contains insert/visual/replace indicators, nvim is NOT in normal mode
        !line_lower.contains("-- insert --")
            && !line_lower.contains("-- visual")
            && !line_lower.contains("-- replace --")
            && !line_lower.contains("-- select --")
    }

    #[allow(clippy::unnecessary_wraps)]
    pub fn handle_key(&mut self, key: KeyEvent) -> Result<Option<Action>> {
        // If nvim is not running, don't handle keys
        if self.status != NvimStatus::Running {
            return Ok(None);
        }

        // Esc: don't consume -- let parent handle it for pane navigation
        // WAIT: Esc must go to nvim for normal mode. The plan says:
        // "Esc is NOT intercepted -- it goes to nvim for normal mode."
        // Focus changes use Tab (already the existing pattern).
        // So we DO forward Esc to nvim.

        // Tab in normal mode: let parent handle focus switching.
        // Tab in insert mode: forward to nvim for indentation.
        if key.code == KeyCode::Tab && key.modifiers.is_empty() && self.is_normal_mode() {
            return Ok(None);
        }

        // Ctrl+R: run query
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('r') {
            let sql = self.get_sql();
            if !sql.is_empty() {
                return Ok(Some(Action::ExecuteQuery(sql)));
            }
            return Ok(Some(Action::StatusMessage("No SQL to execute".to_string())));
        }

        // Ctrl+P: query history
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('p') {
            return Ok(Some(Action::ShowQueryHistory));
        }

        // Ctrl+S: save query
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
            return Ok(Some(Action::SaveQueryToHistory));
        }

        // Ctrl+E: export results
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('e') {
            return Ok(Some(Action::ExportResults));
        }

        // Ctrl+N: new query in normal mode; forward to nvim in insert mode (autocomplete)
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('n') {
            if self.is_normal_mode() {
                return Ok(Some(Action::NewQuery));
            }
            self.write_key(key);
            return Ok(Some(Action::Render));
        }

        // All other keys go to nvim
        self.write_key(key);
        Ok(Some(Action::Render))
    }

    /// Handle nvim exit: reset to `NotStarted` so the editor auto-respawns on next render,
    /// unless nvim has crashed too many times in quick succession.
    /// Clears RPC state so fallback paths are used until RPC reconnects.
    pub fn handle_nvim_exited(&mut self, msg: &str) {
        // Drop the child handle to reap the process
        self.child = None;
        self.rpc_connected = false;
        self.rpc_sync = None;

        // Detect rapid crash loop: if we've spawned 3+ times and the last spawn
        // was less than 3 seconds ago, nvim is crashing immediately on start.
        let crashed_quickly = self
            .last_spawn
            .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(3));

        if self.spawn_attempts >= 3 && crashed_quickly {
            warn!(
                "nvim crashed {} times in quick succession, giving up: {}",
                self.spawn_attempts, msg
            );
            self.status = NvimStatus::Exited(
                "nvim crashed too many times -- switch away and back to retry".to_string(),
            );
        } else {
            self.status = NvimStatus::NotStarted;
        }
    }

    /// Reset spawn attempts counter. Called when switching away from Query mode.
    pub fn reset_spawn_counter(&mut self) {
        self.spawn_attempts = 0;
        self.last_spawn = None;
    }

    /// Store the sync-accessible RPC state after successful connection.
    pub fn set_rpc_sync(&mut self, sync_state: NvimRpcSync) {
        self.rpc_sync = Some(sync_state);
        self.rpc_connected = true;
        info!("NvimEditor: RPC sync state stored");
    }

    /// Check if RPC is connected.
    pub fn has_rpc(&self) -> bool {
        self.rpc_connected
    }

    /// Get the socket path for RPC connection.
    pub fn socket_path(&self) -> &PathBuf {
        &self.socket_path
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Auto-spawn nvim on first render if not running
        if self.status == NvimStatus::NotStarted {
            let inner_cols = area.width.saturating_sub(2);
            let inner_rows = area.height.saturating_sub(4); // 2 borders + 1 hint bar + 1 top padding
            if inner_cols > 0
                && inner_rows > 0
                && let Err(e) = self.spawn(inner_rows, inner_cols)
            {
                error!("Failed to spawn nvim: {}", e);
                self.status = NvimStatus::Exited(format!("Failed to spawn: {e}"));
            }
        }

        // Handle resize
        self.maybe_resize(area);

        let border_color = if self.focused {
            theme::border_focus()
        } else {
            theme::border()
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(theme::bg_surface()).fg(theme::fg()));

        match &self.status {
            NvimStatus::Exited(msg) => {
                let block = block
                    .title(" SQL Editor (nvim exited) ")
                    .title_style(Style::default().fg(theme::accent_dim()));

                let error_text = Paragraph::new(format!(
                    "\n  Embedded nvim exited unexpectedly:\n  {msg}\n\n  Switch away and back to Query mode to restart."
                ))
                .block(block);
                frame.render_widget(error_text, area);
            }
            NvimStatus::NotStarted => {
                let block =
                    block
                        .title(" SQL Editor (starting...) ")
                        .title_style(Style::default().fg(theme::accent_dim()));

                let loading = Paragraph::new("\n  Starting nvim...").block(block);
                frame.render_widget(loading, area);
            }
            NvimStatus::Running => {
                let block =
                    block
                        .title(" SQL Editor ")
                        .title_style(Style::default().fg(theme::accent_dim()));

                let raw_inner = block.inner(area);
                frame.render_widget(block, area);

                if raw_inner.height < 3 {
                    return;
                }

                // Split inner: top padding (1) + nvim content (rest) + hint bar (1)
                let inner_chunks = ratatui::layout::Layout::default()
                    .direction(ratatui::layout::Direction::Vertical)
                    .constraints([
                        ratatui::layout::Constraint::Length(1), // top padding
                        ratatui::layout::Constraint::Min(0),    // nvim content
                        ratatui::layout::Constraint::Length(1), // hint bar
                    ])
                    .split(raw_inner);

                let inner = inner_chunks[1];

                // Read vt100 screen and render cells
                let parser = self.parser.lock().expect("vt100 Parser Mutex poisoned");
                let screen = parser.screen();

                for row in 0..inner.height {
                    for col in 0..inner.width {
                        let cell = screen.cell(row, col);
                        if let Some(cell) = cell {
                            let x = inner.x + col;
                            let y = inner.y + row;

                            if x < inner.x + inner.width && y < inner.y + inner.height {
                                let ch = cell.contents();
                                let display_char = if ch.is_empty() { " " } else { &ch };

                                let fg = vt100_color_to_ratatui(cell.fgcolor());
                                let bg = vt100_color_to_ratatui(cell.bgcolor());

                                let mut style = Style::default().fg(fg).bg(bg);
                                if cell.bold() {
                                    style = style.add_modifier(Modifier::BOLD);
                                }
                                if cell.italic() {
                                    style = style.add_modifier(Modifier::ITALIC);
                                }
                                if cell.underline() {
                                    style = style.add_modifier(Modifier::UNDERLINED);
                                }
                                if cell.inverse() {
                                    style = style.add_modifier(Modifier::REVERSED);
                                }

                                let buf = frame.buffer_mut();
                                if let Some(buf_cell) =
                                    buf.cell_mut(ratatui::layout::Position::new(x, y))
                                {
                                    buf_cell.set_symbol(display_char);
                                    buf_cell.set_style(style);
                                }
                            }
                        }
                    }
                }

                // Render cursor if focused
                if self.focused {
                    let cursor_pos = screen.cursor_position();
                    let cx = inner.x + cursor_pos.1;
                    let cy = inner.y + cursor_pos.0;
                    if cx < inner.x + inner.width && cy < inner.y + inner.height {
                        frame.set_cursor_position(ratatui::layout::Position::new(cx, cy));
                        let buf = frame.buffer_mut();
                        if let Some(buf_cell) = buf.cell_mut(ratatui::layout::Position::new(cx, cy))
                        {
                            buf_cell.set_style(buf_cell.style().add_modifier(Modifier::REVERSED));
                        }
                    }
                }

                // Drop the parser lock before is_normal_mode() which may need it
                drop(parser);

                // Render hint bar inside the pane at the bottom
                let accent = Style::default().fg(theme::accent_dim());
                let dim = Style::default().fg(theme::fg_dim());
                let hint_line = if self.focused {
                    if self.is_normal_mode() {
                        Line::from(vec![
                            Span::styled(" Ctrl+R", accent),
                            Span::styled(": Run  ", dim),
                            Span::styled("Ctrl+N", accent),
                            Span::styled(": New  ", dim),
                            Span::styled("Ctrl+S", accent),
                            Span::styled(": Save  ", dim),
                            Span::styled("Ctrl+L", accent),
                            Span::styled(": Load  ", dim),
                            Span::styled("Ctrl+E", accent),
                            Span::styled(": Export", dim),
                        ])
                    } else {
                        Line::from(vec![
                            Span::styled(" Ctrl+R", accent),
                            Span::styled(": Run  ", dim),
                            Span::styled("Ctrl+S", accent),
                            Span::styled(": Save  ", dim),
                            Span::styled("Ctrl+L", accent),
                            Span::styled(": Load  ", dim),
                            Span::styled("Ctrl+E", accent),
                            Span::styled(": Export", dim),
                        ])
                    }
                } else {
                    Line::from("")
                };
                let hint_bar = Paragraph::new(hint_line)
                    .style(Style::default().bg(theme::bg_surface()).fg(theme::fg()));
                frame.render_widget(hint_bar, inner_chunks[2]);
            }
        }
    }
}

impl Drop for NvimEditor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Convert a vt100 color to a ratatui Color
fn vt100_color_to_ratatui(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(idx) => Color::Indexed(idx),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Stack-allocated key bytes (max 8 bytes, avoids heap allocation per keypress)
struct KeyBytes {
    buf: [u8; 8],
    len: usize,
}

impl KeyBytes {
    fn empty() -> Self {
        Self {
            buf: [0; 8],
            len: 0,
        }
    }

    fn from_slice(s: &[u8]) -> Self {
        let mut buf = [0u8; 8];
        let len = s.len().min(8);
        buf[..len].copy_from_slice(&s[..len]);
        Self { buf, len }
    }

    fn as_slice(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

/// Convert a crossterm `KeyEvent` to raw terminal bytes for the PTY.
/// Returns stack-allocated bytes to avoid heap allocation per keypress.
fn key_to_bytes(key: KeyEvent) -> KeyBytes {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                // Ctrl+letter -> control character (0x01 for 'a', etc.)
                let ctrl_byte = (c.to_ascii_lowercase() as u8)
                    .wrapping_sub(b'a')
                    .wrapping_add(1);
                if alt {
                    KeyBytes::from_slice(&[0x1b, ctrl_byte])
                } else {
                    KeyBytes::from_slice(&[ctrl_byte])
                }
            } else if alt {
                let mut buf = [0u8; 8];
                buf[0] = 0x1b;
                let mut char_buf = [0u8; 4];
                let s = c.encode_utf8(&mut char_buf);
                let bytes = s.as_bytes();
                let len = 1 + bytes.len();
                buf[1..len].copy_from_slice(bytes);
                KeyBytes { buf, len }
            } else {
                let mut buf = [0u8; 8];
                let mut char_buf = [0u8; 4];
                let s = c.encode_utf8(&mut char_buf);
                let bytes = s.as_bytes();
                buf[..bytes.len()].copy_from_slice(bytes);
                KeyBytes {
                    buf,
                    len: bytes.len(),
                }
            }
        }
        KeyCode::Enter => KeyBytes::from_slice(b"\r"),
        KeyCode::Esc => KeyBytes::from_slice(&[0x1b]),
        KeyCode::Backspace => KeyBytes::from_slice(&[0x7f]),
        KeyCode::Tab => KeyBytes::from_slice(b"\t"),
        KeyCode::BackTab => KeyBytes::from_slice(&[0x1b, b'[', b'Z']),
        KeyCode::Up => {
            if ctrl {
                KeyBytes::from_slice(b"\x1b[1;5A")
            } else if shift {
                KeyBytes::from_slice(b"\x1b[1;2A")
            } else {
                KeyBytes::from_slice(b"\x1b[A")
            }
        }
        KeyCode::Down => {
            if ctrl {
                KeyBytes::from_slice(b"\x1b[1;5B")
            } else if shift {
                KeyBytes::from_slice(b"\x1b[1;2B")
            } else {
                KeyBytes::from_slice(b"\x1b[B")
            }
        }
        KeyCode::Right => {
            if ctrl {
                KeyBytes::from_slice(b"\x1b[1;5C")
            } else if shift {
                KeyBytes::from_slice(b"\x1b[1;2C")
            } else {
                KeyBytes::from_slice(b"\x1b[C")
            }
        }
        KeyCode::Left => {
            if ctrl {
                KeyBytes::from_slice(b"\x1b[1;5D")
            } else if shift {
                KeyBytes::from_slice(b"\x1b[1;2D")
            } else {
                KeyBytes::from_slice(b"\x1b[D")
            }
        }
        KeyCode::Home => KeyBytes::from_slice(b"\x1b[H"),
        KeyCode::End => KeyBytes::from_slice(b"\x1b[F"),
        KeyCode::PageUp => KeyBytes::from_slice(b"\x1b[5~"),
        KeyCode::PageDown => KeyBytes::from_slice(b"\x1b[6~"),
        KeyCode::Insert => KeyBytes::from_slice(b"\x1b[2~"),
        KeyCode::Delete => KeyBytes::from_slice(b"\x1b[3~"),
        KeyCode::F(n) => match n {
            1 => KeyBytes::from_slice(b"\x1bOP"),
            2 => KeyBytes::from_slice(b"\x1bOQ"),
            3 => KeyBytes::from_slice(b"\x1bOR"),
            4 => KeyBytes::from_slice(b"\x1bOS"),
            5 => KeyBytes::from_slice(b"\x1b[15~"),
            6 => KeyBytes::from_slice(b"\x1b[17~"),
            7 => KeyBytes::from_slice(b"\x1b[18~"),
            8 => KeyBytes::from_slice(b"\x1b[19~"),
            9 => KeyBytes::from_slice(b"\x1b[20~"),
            10 => KeyBytes::from_slice(b"\x1b[21~"),
            11 => KeyBytes::from_slice(b"\x1b[23~"),
            12 => KeyBytes::from_slice(b"\x1b[24~"),
            _ => KeyBytes::empty(),
        },
        _ => KeyBytes::empty(),
    }
}

/// Find the bundled init.lua file
fn find_init_lua() -> PathBuf {
    // Try relative to executable
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        let candidate = exe_dir.join("../runtime/init.lua");
        if candidate.exists() {
            return candidate;
        }
        let candidate = exe_dir.join("runtime/init.lua");
        if candidate.exists() {
            return candidate;
        }
    }

    // Try relative to CWD (for development)
    let cwd_candidate = PathBuf::from("runtime/init.lua");
    if cwd_candidate.exists() {
        // Return absolute path
        if let Ok(abs) = std::fs::canonicalize(&cwd_candidate) {
            return abs;
        }
        return cwd_candidate;
    }

    // Try the cargo manifest dir (compile time)
    let manifest_candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime/init.lua");
    if manifest_candidate.exists() {
        return manifest_candidate;
    }

    // Fallback -- will cause nvim to start with defaults
    warn!("Could not find bundled init.lua, nvim will use defaults");
    PathBuf::from("/dev/null")
}

/// Check if nvim is available on PATH
pub fn check_nvim_available() -> Result<(), String> {
    match std::process::Command::new("nvim").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            if let Some(first_line) = version.lines().next() {
                info!("Found {}", first_line);
            }
            Ok(())
        }
        Ok(_) => Err("nvim found but --version failed".to_string()),
        Err(e) => Err(format!(
            "nvim not found on PATH. Install neovim to use Query mode. Error: {e}"
        )),
    }
}
