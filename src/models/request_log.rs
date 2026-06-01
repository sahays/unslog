//! `request_log` model — per-request audit + cost accounting.
//!
//! Mirrors the schema introduced in `migrations/0005_request_log.sql`:
//! one row per metered request, an enum `kind` matching the CHECK
//! constraint, and a small projected `UsageWindow` used by the cap-check
//! and the sidebar usage bar.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::types::Json;

/// Cost category. Mirrors the `kind` CHECK in 0005 1:1.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RequestKind {
    /// Pure read / navigation — no LLM, no audio, free.
    PageNav,
    /// Hits an LLM (critique, summary, story-chat, pitch-chat, research, …).
    Llm,
    /// Synthesises audio (TTS).
    Tts,
    /// Speech-to-text (transcribe).
    Stt,
    /// Asset upload + extraction pipeline.
    Upload,
    /// Anything else we want logged but not categorized.
    Other,
}

impl RequestKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RequestKind::PageNav => "page_nav",
            RequestKind::Llm => "llm",
            RequestKind::Tts => "tts",
            RequestKind::Stt => "stt",
            RequestKind::Upload => "upload",
            RequestKind::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "page_nav" => Some(RequestKind::PageNav),
            "llm" => Some(RequestKind::Llm),
            "tts" => Some(RequestKind::Tts),
            "stt" => Some(RequestKind::Stt),
            "upload" => Some(RequestKind::Upload),
            "other" => Some(RequestKind::Other),
            _ => None,
        }
    }
}

/// One row of `request_log`. Forensic projection — the in-app code rarely
/// needs to read these back; provided for tests and a future admin view.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RequestLogEntry {
    pub id: i64,
    pub user_id: String,
    pub ts: DateTime<Utc>,
    pub method: String,
    pub path: String,
    pub status: i16,
    pub duration_ms: Option<i32>,
    pub kind: RequestKind,
    pub cost_units: i32,
    pub llm_input_tokens: Option<i32>,
    pub llm_output_tokens: Option<i32>,
    pub llm_cost_usd_micros: Option<i64>,
    pub meta: Json<serde_json::Value>,
}

/// Snapshot of one user's last-24h spend. `cap` is `u64::MAX` for master
/// (unlimited) so the rendering layer can treat the bar uniformly.
#[derive(Debug, Clone, Copy)]
pub struct UsageWindow {
    pub used: u64,
    pub cap: u64,
}

impl UsageWindow {
    /// 0..=100 percentage used. Saturates at 100. Master (`cap = u64::MAX`)
    /// always reports 0 so the bar reads "empty" instead of "full".
    pub fn pct(&self) -> u8 {
        if self.cap == 0 || self.cap == u64::MAX {
            return 0;
        }
        let pct = self.used.saturating_mul(100) / self.cap;
        pct.min(100) as u8
    }
}
