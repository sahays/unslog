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
    /// Postgres connection URL — the app's sole datastore. Defaults to the
    /// project-scoped `unslog-pg` container started by `scripts/dev-up.sh`.
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

    // ── Multi-user (Phase 1+) ─────────────────────────────────────────────
    /// Name of the session cookie. Defaults to `__Host-session`; the prefix
    /// forces Secure + same-origin + path=/ at the browser layer.
    pub session_cookie_name: String,
    /// Name of the CSRF double-submit cookie. Defaults to `__Host-csrf`.
    pub csrf_cookie_name: String,
    /// Brute-force throttle: max `/login` attempts per IP within
    /// `login_throttle_window_secs`. Defaults to 5.
    pub login_throttle_max_attempts: u32,
    /// Window (seconds) over which login attempts are counted. Defaults to 300.
    pub login_throttle_window_secs: u64,
    /// Daily request budget for Pro-tier users. Master account is exempt.
    pub pro_request_cap_daily: u32,
    /// Upper bound an admin may bump a single Pro user's daily cap to.
    pub pro_max_request_cap_daily: u32,
    /// Dev-only: drop the `Secure` cookie attribute so cookies work over
    /// plain `http://localhost`. Must remain `false` in production.
    pub dev_insecure: bool,
}

impl AppConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            host: env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
            port: env::var("PORT")
                .unwrap_or_else(|_| "3000".to_string())
                .parse()?,
            // Soft default mirrors `scripts/dev-up.sh` so a fresh checkout
            // boots against the project-scoped `unslog-pg` container
            // without a `.env` change.
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
            // Multi-user (Phase 1+) — defaults are safe-for-production except
            // `DEV_INSECURE`, which must stay false anywhere with real TLS.
            session_cookie_name: env_string("SESSION_COOKIE_NAME", "__Host-session"),
            csrf_cookie_name: env_string("CSRF_COOKIE_NAME", "__Host-csrf"),
            login_throttle_max_attempts: env_parse("LOGIN_THROTTLE_MAX_ATTEMPTS", 5),
            login_throttle_window_secs: env_parse("LOGIN_THROTTLE_WINDOW_SECS", 300),
            pro_request_cap_daily: env_parse("PRO_REQUEST_CAP_DAILY", 500),
            pro_max_request_cap_daily: env_parse("PRO_MAX_REQUEST_CAP_DAILY", 2000),
            dev_insecure: env_bool("DEV_INSECURE", false),
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

/// Read an env var or fall back to `default`. Blank string is treated as
/// missing so a checked-in `.env.example` placeholder doesn't override the
/// in-code default.
fn env_string(key: &str, default: &str) -> String {
    env::var(key)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    env::var(key)
        .ok()
        .and_then(|s| s.trim().parse::<T>().ok())
        .unwrap_or(default)
}

fn env_bool(key: &str, default: bool) -> bool {
    env::var(key)
        .ok()
        .map(|s| {
            matches!(
                s.trim().to_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}
