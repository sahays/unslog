//! Store for the Postgres `sessions` table — practice-session shells.
//!
//! Read/write surface:
//! * `get` / `find_or_404` — lookups.
//! * `set_current_question` — flips that the advance/next handlers used to inline.
//! * `end` — flip status to `ended`, stamp `ended_at`, clear current question.
//! * `delete_cascade` — owner-scoped DELETE; the FK CASCADEs in migration
//!   0001 wipe evaluations + summaries.
//! * `list_active` / `list_for_company` — projected reads for the practice
//!   page and the company show page.
//! * `count_active` — index-page summary.
//! * `toggle_voice` — small flip used by the voice toggle handler.
//! * `insert` — used by `services::session::start`.
//!
//! Every read filters by `owner_id`; every write sets it. The handler
//! layer threads `CurrentUser::id` through.

use chrono::{DateTime, Utc};
use sqlx::types::Json;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{ModelSnapshot, PromptSnapshot, Role, Session, SessionStatus};

/// Columns selected by every full-row read, in the order [`SessionRow`]
/// expects. Centralized so a schema add only edits one place. We use the
/// plain column names (no `AS "name: Type"` annotation) because these SQL
/// fragments are interpolated into `sqlx::query_as` calls, not the
/// `query_as!` macro — the macro-only annotation would otherwise become
/// part of the actual returned column name and break `FromRow` lookup.
const SESSION_COLS: &str = r#"id, owner_id, company_id, role,
    selected_company_ids,
    curated_question_ids,
    focus_line, started_at, ended_at, status,
    model_snapshot,
    prompt_snapshot,
    voice_critique_enabled,
    current_question_id, current_question_text, current_question_audio_path"#;

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: String,
    owner_id: String,
    company_id: String,
    role: String,
    selected_company_ids: Json<Vec<String>>,
    curated_question_ids: Json<Vec<String>>,
    focus_line: String,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    status: String,
    model_snapshot: Json<ModelSnapshot>,
    prompt_snapshot: Json<PromptSnapshot>,
    voice_critique_enabled: bool,
    current_question_id: Option<String>,
    current_question_text: Option<String>,
    current_question_audio_path: Option<String>,
}

impl SessionRow {
    fn try_into_session(self) -> Result<Session, AppError> {
        let role = Role::parse(&self.role).ok_or_else(|| {
            AppError::Other(anyhow::anyhow!(
                "sessions.role {:?} not in CHECK set",
                self.role
            ))
        })?;
        let status = SessionStatus::parse(&self.status).ok_or_else(|| {
            AppError::Other(anyhow::anyhow!(
                "sessions.status {:?} not in CHECK set",
                self.status
            ))
        })?;
        Ok(Session {
            id: self.id,
            owner_id: self.owner_id,
            company_id: self.company_id,
            role,
            selected_company_ids: self.selected_company_ids.0,
            curated_question_ids: self.curated_question_ids.0,
            focus_line: self.focus_line,
            started_at: self.started_at,
            ended_at: self.ended_at,
            status,
            model_snapshot: self.model_snapshot.0,
            prompt_snapshot: self.prompt_snapshot.0,
            voice_critique_enabled: self.voice_critique_enabled,
            current_question_id: self.current_question_id,
            current_question_text: self.current_question_text,
            current_question_audio_path: self.current_question_audio_path,
        })
    }
}

// ── Reads ────────────────────────────────────────────────────────────────

pub async fn get(pool: &PgPool, owner_id: &str, id: &str) -> Result<Option<Session>, AppError> {
    let sql = format!("SELECT {SESSION_COLS} FROM sessions WHERE owner_id = $1 AND id = $2");
    let row: Option<SessionRow> = sqlx::query_as(&sql)
        .bind(owner_id)
        .bind(id)
        .fetch_optional(pool)
        .await?;
    row.map(SessionRow::try_into_session).transpose()
}

pub async fn find_or_404(pool: &PgPool, owner_id: &str, id: &str) -> Result<Session, AppError> {
    get(pool, owner_id, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("session {id}")))
}

/// Active sessions, newest-first. Drives the `/practice` "in progress" rail.
pub async fn list_active(pool: &PgPool, owner_id: &str) -> Result<Vec<Session>, AppError> {
    let sql = format!(
        "SELECT {SESSION_COLS} FROM sessions
         WHERE owner_id = $1 AND status = $2
         ORDER BY started_at DESC"
    );
    let rows: Vec<SessionRow> = sqlx::query_as(&sql)
        .bind(owner_id)
        .bind(SessionStatus::Active.as_str())
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(SessionRow::try_into_session).collect()
}

/// Sessions for one company, newest-first. Drives the company show page.
pub async fn list_for_company(
    pool: &PgPool,
    owner_id: &str,
    company_id: &str,
) -> Result<Vec<Session>, AppError> {
    let sql = format!(
        "SELECT {SESSION_COLS} FROM sessions
         WHERE owner_id = $1 AND company_id = $2
         ORDER BY started_at DESC"
    );
    let rows: Vec<SessionRow> = sqlx::query_as(&sql)
        .bind(owner_id)
        .bind(company_id)
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(SessionRow::try_into_session).collect()
}

/// Cheap count of active sessions — for the home page status card.
pub async fn count_active(pool: &PgPool, owner_id: &str) -> Result<u64, AppError> {
    let n: i64 = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "count!" FROM sessions
           WHERE owner_id = $1 AND status = 'active'"#,
        owner_id,
    )
    .fetch_one(pool)
    .await?;
    Ok(n as u64)
}

// ── Writes ───────────────────────────────────────────────────────────────

/// Insert a freshly-built session row. The caller sets `session.owner_id`
/// and mints `session.id` via `Session::new_id`.
pub async fn insert(pool: &PgPool, session: &Session) -> Result<(), AppError> {
    sqlx::query(
        r#"INSERT INTO sessions
           (id, owner_id, company_id, role,
            selected_company_ids, curated_question_ids, focus_line,
            started_at, ended_at, status,
            model_snapshot, prompt_snapshot, voice_critique_enabled,
            current_question_id, current_question_text, current_question_audio_path)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)"#,
    )
    .bind(&session.id)
    .bind(&session.owner_id)
    .bind(&session.company_id)
    .bind(session.role.as_str())
    .bind(Json(&session.selected_company_ids))
    .bind(Json(&session.curated_question_ids))
    .bind(&session.focus_line)
    .bind(session.started_at)
    .bind(session.ended_at)
    .bind(session.status.as_str())
    .bind(Json(&session.model_snapshot))
    .bind(Json(&session.prompt_snapshot))
    .bind(session.voice_critique_enabled)
    .bind(&session.current_question_id)
    .bind(&session.current_question_text)
    .bind(&session.current_question_audio_path)
    .execute(pool)
    .await?;
    Ok(())
}

/// Persist the next curated question on the active session. Owner-scoped
/// so a malicious id doesn't let an attacker write into another owner's row.
pub async fn set_current_question(
    pool: &PgPool,
    owner_id: &str,
    session_id: &str,
    qid: &str,
    qtext: &str,
    audio_path: Option<&str>,
) -> Result<(), AppError> {
    sqlx::query(
        r#"UPDATE sessions
           SET current_question_id = $3,
               current_question_text = $4,
               current_question_audio_path = $5
           WHERE owner_id = $1 AND id = $2"#,
    )
    .bind(owner_id)
    .bind(session_id)
    .bind(qid)
    .bind(qtext)
    .bind(audio_path)
    .execute(pool)
    .await?;
    Ok(())
}

/// Flip status to `ended`, stamp `ended_at`, and clear current_question_*.
/// Idempotent against the projection callers care about.
pub async fn end(pool: &PgPool, owner_id: &str, session_id: &str) -> Result<(), AppError> {
    sqlx::query(
        r#"UPDATE sessions
           SET status = $3,
               ended_at = NOW(),
               current_question_id = NULL,
               current_question_text = NULL
           WHERE owner_id = $1 AND id = $2"#,
    )
    .bind(owner_id)
    .bind(session_id)
    .bind(SessionStatus::Ended.as_str())
    .execute(pool)
    .await?;
    Ok(())
}

/// Flip `voice_critique_enabled` and return the new value.
pub async fn toggle_voice(pool: &PgPool, session: &Session) -> Result<bool, AppError> {
    let new_value = !session.voice_critique_enabled;
    sqlx::query(
        r#"UPDATE sessions SET voice_critique_enabled = $3
           WHERE owner_id = $1 AND id = $2"#,
    )
    .bind(&session.owner_id)
    .bind(&session.id)
    .bind(new_value)
    .execute(pool)
    .await?;
    Ok(new_value)
}

/// Delete a session row. The Postgres FK CASCADEs (migration 0001) drop
/// the dependent evaluations + summary in the same transaction. Returns
/// the number of session rows deleted (0 or 1) so callers can log it.
pub async fn delete_cascade(
    pool: &PgPool,
    owner_id: &str,
    session_id: &str,
) -> Result<u64, AppError> {
    let res = sqlx::query("DELETE FROM sessions WHERE owner_id = $1 AND id = $2")
        .bind(owner_id)
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}
