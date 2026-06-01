use std::env;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// Human-readable single-line records — best for `tail -f` next to your browser.
    Compact,
    /// One JSON object per line — feed this into jq, Loki, Datadog, etc.
    Json,
}

impl LogFormat {
    fn from_env() -> Self {
        match env::var("LOG_FORMAT")
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "json" => Self::Json,
            _ => Self::Compact,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub mongo_uri: String,
    pub mongo_db: String,
    /// Postgres connection URL. Read at boot but unused until the
    /// Mongo → Postgres code swap lands in a later sub-phase. No default —
    /// once Postgres becomes required, `from_env` will fail fast if missing.
    pub database_url: String,
    pub openrouter_api_key: String,
    pub data_dir: String,
    pub log_format: LogFormat,
    /// Subdirectory under `data_dir` for log files. Defaults to "logs".
    pub log_dir: String,
    /// Optional HTTP-Referer to send to OpenRouter (env: `UNSLOG_REFERER`).
    /// `None` (unset/blank) means omit the header — avoids leaking a
    /// personal repo URL in attribution headers on shared/public builds.
    pub referer: Option<String>,
}

impl AppConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            host: env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
            port: env::var("PORT")
                .unwrap_or_else(|_| "3000".to_string())
                .parse()?,
            mongo_uri: env::var("MONGO_URI")
                .unwrap_or_else(|_| "mongodb://localhost:27017".to_string()),
            mongo_db: env::var("MONGO_DB").unwrap_or_else(|_| "behavioral_coach".to_string()),
            // Soft default mirrors `scripts/dev-up.sh` so a fresh checkout
            // boots against the project-scoped `unslog-pg` container without
            // a `.env` change. Will switch to hard-fail when the code swap
            // lands.
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://unslog:unslog@localhost:5432/unslog".to_string()),
            openrouter_api_key: env::var("OPENROUTER_API_KEY").unwrap_or_default(),
            data_dir: env::var("DATA_DIR").unwrap_or_else(|_| "./data".to_string()),
            log_format: LogFormat::from_env(),
            log_dir: env::var("LOG_DIR").unwrap_or_else(|_| "logs".to_string()),
            referer: env::var("UNSLOG_REFERER")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        })
    }

    pub fn addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub fn openrouter_configured(&self) -> bool {
        !self.openrouter_api_key.trim().is_empty()
    }

    pub fn log_path(&self) -> std::path::PathBuf {
        std::path::Path::new(&self.data_dir).join(&self.log_dir)
    }
}
