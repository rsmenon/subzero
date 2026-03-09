use std::collections::HashMap;
use std::path::{Path, PathBuf};

use color_eyre::Result;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct SnowflakeConnection {
    pub account: Option<String>,
    pub user: Option<String>,
    pub warehouse: Option<String>,
    pub database: Option<String>,
    pub role: Option<String>,
    pub schema: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SnowflakeConfig {
    #[serde(default)]
    pub default_connection_name: Option<String>,
    #[serde(default)]
    pub connections: HashMap<String, SnowflakeConnection>,
}

#[derive(Debug, Clone, Deserialize)]
struct ConnectionsFile {
    #[serde(default)]
    pub default_connection_name: Option<String>,
    #[serde(flatten)]
    pub connections: HashMap<String, SnowflakeConnection>,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub snowflake: Option<SnowflakeConfig>,
    pub connection_name: Option<String>,
    pub log_path: PathBuf,
    pub cache_dir: PathBuf,
    pub app_dir: PathBuf,
}

impl AppConfig {
    #[allow(clippy::unnecessary_wraps)]
    pub fn load() -> Result<Self> {
        let home = dirs_home();
        let snowflake_dir = home.join(".snowflake");

        // Try connections.toml first, then config.toml
        let snowflake = Self::try_load_connections(&snowflake_dir)
            .or_else(|_| Self::try_load_config(&snowflake_dir))
            .ok();

        let connection_name = snowflake.as_ref().and_then(|c| {
            c.default_connection_name.clone().or_else(|| {
                // Fall back to a connection named "default", then to the first connection
                let mut names: Vec<&String> = c.connections.keys().collect();
                names.sort();
                names
                    .iter()
                    .find(|n| n.as_str() == "default")
                    .or_else(|| names.first())
                    .map(|n| (*n).clone())
            })
        });

        let app_dir = app_dir(&home);
        let cache_dir = app_dir.join("data");
        let log_path = app_dir.join("sz.log");

        // Ensure directories exist
        std::fs::create_dir_all(&app_dir).ok();
        std::fs::create_dir_all(&cache_dir).ok();

        Ok(Self {
            snowflake,
            connection_name,
            log_path,
            cache_dir,
            app_dir,
        })
    }

    fn try_load_connections(snowflake_dir: &Path) -> Result<SnowflakeConfig> {
        let path = snowflake_dir.join("connections.toml");
        let content = std::fs::read_to_string(&path)?;
        let file: ConnectionsFile = toml::from_str(&content)?;
        Ok(SnowflakeConfig {
            default_connection_name: file.default_connection_name,
            connections: file.connections,
        })
    }

    fn try_load_config(snowflake_dir: &Path) -> Result<SnowflakeConfig> {
        let path = snowflake_dir.join("config.toml");
        let content = std::fs::read_to_string(&path)?;
        let config: SnowflakeConfig = toml::from_str(&content)?;
        Ok(config)
    }
}

fn dirs_home() -> PathBuf {
    directories::BaseDirs::new().map_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())), |d| d.home_dir().to_path_buf())
}

/// App directory: ~/.subzero on macOS/Linux, %APPDATA%\subzero on Windows.
fn app_dir(home: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        std::env::var("APPDATA")
            .map(|d| PathBuf::from(d).join("subzero"))
            .unwrap_or_else(|_| home.join(".subzero"))
    }
    #[cfg(not(windows))]
    {
        home.join(".subzero")
    }
}
