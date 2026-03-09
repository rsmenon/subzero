use std::io::{self, stdout};

use color_eyre::Result;
use crossterm::{
    event::{KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, prelude::CrosstermBackend};

pub type Tui = Terminal<CrosstermBackend<io::Stdout>>;

pub fn init() -> Result<Tui> {
    // Set up panic hook to restore terminal on panic
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = restore();
        original_hook(panic_info);
    }));

    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    // Enable kitty keyboard protocol for disambiguated key events (e.g. Ctrl+Enter).
    // Silently ignored by terminals that don't support it.
    let _ = execute!(
        stdout(),
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    );
    let backend = CrosstermBackend::new(stdout());
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

pub fn restore() -> Result<()> {
    let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;
    Ok(())
}
