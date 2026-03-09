use std::sync::{OnceLock, RwLock};

use ratatui::style::Color;
use serde::{Deserialize, Serialize};

/// Serializable theme configuration. Stored as hex strings in YAML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    pub bg: String,
    pub bg_surface: String,
    pub bg_highlight: String,
    pub fg: String,
    pub fg_dim: String,
    pub fg_mid: String,
    pub fg_bright: String,
    pub accent: String,
    pub accent_dim: String,
    pub tab_active_bg: String,
    pub tab_inactive_bg: String,
    pub border: String,
    pub border_focus: String,
    pub green: String,
    pub red: String,
    pub yellow: String,
    pub chart_purple: String,
    pub chart_mint: String,
    pub dialog_bg: String,
    pub dialog_border: String,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            bg: "#282828".to_string(),
            bg_surface: "#282828".to_string(),
            bg_highlight: "#3c3836".to_string(),
            fg: "#d5c4a1".to_string(),
            fg_dim: "#928374".to_string(),
            fg_mid: "#a89984".to_string(),
            fg_bright: "#fbf1c7".to_string(),
            accent: "#83a598".to_string(),
            accent_dim: "#458588".to_string(),
            tab_active_bg: "#3c3836".to_string(),
            tab_inactive_bg: "#282828".to_string(),
            border: "#3c3836".to_string(),
            border_focus: "#458588".to_string(),
            green: "#b8bb26".to_string(),
            red: "#fb4934".to_string(),
            yellow: "#fabd2f".to_string(),
            chart_purple: "#a078d2".to_string(),
            chart_mint: "#64c882".to_string(),
            dialog_bg: "#32302f".to_string(),
            dialog_border: "#458588".to_string(),
        }
    }
}

/// Runtime color values parsed from `ThemeConfig`.
struct ThemeColors {
    bg: Color,
    bg_surface: Color,
    bg_highlight: Color,
    fg: Color,
    fg_dim: Color,
    fg_mid: Color,
    fg_bright: Color,
    accent: Color,
    accent_dim: Color,
    tab_active_bg: Color,
    tab_inactive_bg: Color,
    border: Color,
    border_focus: Color,
    green: Color,
    red: Color,
    yellow: Color,
    chart_purple: Color,
    chart_mint: Color,
    dialog_bg: Color,
    dialog_border: Color,
}

impl ThemeColors {
    fn from_config(config: &ThemeConfig) -> Result<Self, String> {
        Ok(Self {
            bg: parse_hex(&config.bg).ok_or_else(|| format!("Invalid bg: {}", config.bg))?,
            bg_surface: parse_hex(&config.bg_surface)
                .ok_or_else(|| format!("Invalid bg_surface: {}", config.bg_surface))?,
            bg_highlight: parse_hex(&config.bg_highlight)
                .ok_or_else(|| format!("Invalid bg_highlight: {}", config.bg_highlight))?,
            fg: parse_hex(&config.fg).ok_or_else(|| format!("Invalid fg: {}", config.fg))?,
            fg_dim: parse_hex(&config.fg_dim)
                .ok_or_else(|| format!("Invalid fg_dim: {}", config.fg_dim))?,
            fg_mid: parse_hex(&config.fg_mid)
                .ok_or_else(|| format!("Invalid fg_mid: {}", config.fg_mid))?,
            fg_bright: parse_hex(&config.fg_bright)
                .ok_or_else(|| format!("Invalid fg_bright: {}", config.fg_bright))?,
            accent: parse_hex(&config.accent)
                .ok_or_else(|| format!("Invalid accent: {}", config.accent))?,
            accent_dim: parse_hex(&config.accent_dim)
                .ok_or_else(|| format!("Invalid accent_dim: {}", config.accent_dim))?,
            tab_active_bg: parse_hex(&config.tab_active_bg)
                .ok_or_else(|| format!("Invalid tab_active_bg: {}", config.tab_active_bg))?,
            tab_inactive_bg: parse_hex(&config.tab_inactive_bg)
                .ok_or_else(|| format!("Invalid tab_inactive_bg: {}", config.tab_inactive_bg))?,
            border: parse_hex(&config.border)
                .ok_or_else(|| format!("Invalid border: {}", config.border))?,
            border_focus: parse_hex(&config.border_focus)
                .ok_or_else(|| format!("Invalid border_focus: {}", config.border_focus))?,
            green: parse_hex(&config.green)
                .ok_or_else(|| format!("Invalid green: {}", config.green))?,
            red: parse_hex(&config.red).ok_or_else(|| format!("Invalid red: {}", config.red))?,
            yellow: parse_hex(&config.yellow)
                .ok_or_else(|| format!("Invalid yellow: {}", config.yellow))?,
            chart_purple: parse_hex(&config.chart_purple)
                .ok_or_else(|| format!("Invalid chart_purple: {}", config.chart_purple))?,
            chart_mint: parse_hex(&config.chart_mint)
                .ok_or_else(|| format!("Invalid chart_mint: {}", config.chart_mint))?,
            dialog_bg: parse_hex(&config.dialog_bg)
                .ok_or_else(|| format!("Invalid dialog_bg: {}", config.dialog_bg))?,
            dialog_border: parse_hex(&config.dialog_border)
                .ok_or_else(|| format!("Invalid dialog_border: {}", config.dialog_border))?,
        })
    }
}

/// Global runtime theme. Initialized once at startup, can be updated by `apply()`.
static THEME: OnceLock<RwLock<ThemeColors>> = OnceLock::new();

fn colors() -> std::sync::RwLockReadGuard<'static, ThemeColors> {
    THEME
        .get()
        .expect("theme not initialized — call theme::load() before rendering")
        .read()
        .expect("theme RwLock poisoned")
}

// Public accessor functions (replace the old pub const values)
pub fn bg() -> Color {
    colors().bg
}
pub fn bg_surface() -> Color {
    colors().bg_surface
}
pub fn bg_highlight() -> Color {
    colors().bg_highlight
}
pub fn fg() -> Color {
    colors().fg
}
pub fn fg_dim() -> Color {
    colors().fg_dim
}
pub fn fg_mid() -> Color {
    colors().fg_mid
}
pub fn fg_bright() -> Color {
    colors().fg_bright
}
pub fn accent() -> Color {
    colors().accent
}
pub fn accent_dim() -> Color {
    colors().accent_dim
}
pub fn tab_active_bg() -> Color {
    colors().tab_active_bg
}
pub fn tab_inactive_bg() -> Color {
    colors().tab_inactive_bg
}
pub fn border() -> Color {
    colors().border
}
pub fn border_focus() -> Color {
    colors().border_focus
}
pub fn green() -> Color {
    colors().green
}
pub fn red() -> Color {
    colors().red
}
pub fn yellow() -> Color {
    colors().yellow
}
pub fn chart_purple() -> Color {
    colors().chart_purple
}
pub fn chart_mint() -> Color {
    colors().chart_mint
}
pub fn dialog_bg() -> Color {
    colors().dialog_bg
}
pub fn dialog_border() -> Color {
    colors().dialog_border
}

/// Parse a hex string like "#282828" or "282828" into `Color::Rgb`.
/// Returns None if invalid.
pub fn parse_hex(hex: &str) -> Option<Color> {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

/// Format a `Color::Rgb` as "#RRGGBB". Returns None for non-Rgb colors.
pub fn color_to_hex(color: Color) -> Option<String> {
    if let Color::Rgb(r, g, b) = color {
        Some(format!("#{r:02x}{g:02x}{b:02x}"))
    } else {
        None
    }
}

/// Load theme from file, falling back to defaults. Called once at startup.
pub fn load(path: &std::path::Path) {
    let config = load_config(path).unwrap_or_default();
    let theme_colors = ThemeColors::from_config(&config).unwrap_or_else(|_| {
        ThemeColors::from_config(&ThemeConfig::default())
            .expect("default theme config must be valid")
    });
    // OnceLock::set will fail silently if already initialized (e.g., in tests)
    let _ = THEME.set(RwLock::new(theme_colors));
}

/// Apply a new `ThemeConfig` at runtime. Replaces all colors.
/// Returns Err if any hex value is invalid.
pub fn apply(config: &ThemeConfig) -> Result<(), String> {
    let new_colors = ThemeColors::from_config(config)?;
    let lock = THEME
        .get()
        .ok_or_else(|| "theme not initialized".to_string())?;
    let mut guard = lock.write().map_err(|e| format!("RwLock poisoned: {e}"))?;
    *guard = new_colors;
    Ok(())
}

/// Save `ThemeConfig` to a YAML file.
pub fn save(config: &ThemeConfig, path: &std::path::Path) -> std::io::Result<()> {
    let yaml = serde_yaml::to_string(config)
        .map_err(std::io::Error::other)?;
    std::fs::write(path, yaml)
}

/// Load `ThemeConfig` from a YAML file. Returns None if file missing or invalid.
pub fn load_config(path: &std::path::Path) -> Option<ThemeConfig> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_yaml::from_str(&content).ok()
}

/// Return the current theme config by reading the runtime colors.
pub fn current_config() -> ThemeConfig {
    let c = colors();
    ThemeConfig {
        bg: color_to_hex(c.bg).unwrap_or_default(),
        bg_surface: color_to_hex(c.bg_surface).unwrap_or_default(),
        bg_highlight: color_to_hex(c.bg_highlight).unwrap_or_default(),
        fg: color_to_hex(c.fg).unwrap_or_default(),
        fg_dim: color_to_hex(c.fg_dim).unwrap_or_default(),
        fg_mid: color_to_hex(c.fg_mid).unwrap_or_default(),
        fg_bright: color_to_hex(c.fg_bright).unwrap_or_default(),
        accent: color_to_hex(c.accent).unwrap_or_default(),
        accent_dim: color_to_hex(c.accent_dim).unwrap_or_default(),
        tab_active_bg: color_to_hex(c.tab_active_bg).unwrap_or_default(),
        tab_inactive_bg: color_to_hex(c.tab_inactive_bg).unwrap_or_default(),
        border: color_to_hex(c.border).unwrap_or_default(),
        border_focus: color_to_hex(c.border_focus).unwrap_or_default(),
        green: color_to_hex(c.green).unwrap_or_default(),
        red: color_to_hex(c.red).unwrap_or_default(),
        yellow: color_to_hex(c.yellow).unwrap_or_default(),
        chart_purple: color_to_hex(c.chart_purple).unwrap_or_default(),
        chart_mint: color_to_hex(c.chart_mint).unwrap_or_default(),
        dialog_bg: color_to_hex(c.dialog_bg).unwrap_or_default(),
        dialog_border: color_to_hex(c.dialog_border).unwrap_or_default(),
    }
}
