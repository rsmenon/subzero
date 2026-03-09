use crate::config::AppConfig;

#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    pub connection_name: Option<String>,
}

impl ConnectionConfig {
    pub fn from_app_config(config: &AppConfig) -> Self {
        Self {
            connection_name: config.connection_name.clone(),
        }
    }

    /// Build the base snow CLI args
    pub fn base_args(&self) -> Vec<String> {
        let mut args = vec![
            "sql".to_string(),
            "--format".to_string(),
            "JSON_EXT".to_string(),
            "--silent".to_string(),
            "--enhanced-exit-codes".to_string(),
        ];

        if let Some(ref conn) = self.connection_name {
            args.push("-c".to_string());
            args.push(conn.clone());
        }

        args
    }
}
