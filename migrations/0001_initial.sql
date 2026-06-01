-- 0001_initial.sql
-- Initial relational schema mirroring the existing MongoDB collections.
-- No owner_id yet — that lands in 0003 once users (0002) are in place.
--
-- ID conventions:
--   * Non-catalog rows use 9-char "{3-prefix}{6 lowercase alphanum}" ids.
--   * Catalog rows (categories, pitches) use stable semantic slugs and are
--     exempt from the format check.
--   * Enum-like columns are TEXT + CHECK to match Rust serde
--     `rename_all = "snake_case"` exactly.
--   * Embedded BSON arrays / objects become JSONB.
--   * FK columns get explicit btree indexes (the FK itself does not).

-- ── categories ──────────────────────────────────────────────────────────
-- Catalog: semantic slug id (e.g. "ownership", "bias-for-action"). No
-- format check; not user-owned.
CREATE TABLE categories (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ── companies ───────────────────────────────────────────────────────────
-- research_packet is stored inline as JSONB on the row (it has 1:1 ownership
-- by the company and no independent query needs).
CREATE TABLE companies (
    id              TEXT PRIMARY KEY CHECK (id ~ '^com[a-z0-9]{6}$'),
    name            TEXT NOT NULL,
    role            TEXT NOT NULL,
    canonical_role  TEXT NOT NULL CHECK (canonical_role IN (
        'solutions_architect',
        'software_engineer',
        'engineering_manager',
        'program_manager',
        'product_manager'
    )),
    research_packet JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ── questions ───────────────────────────────────────────────────────────
-- company_id is nullable (role-only questions exist). Cascade on company
-- deletion mirrors `services::questions::delete_for_company`.
CREATE TABLE questions (
    id          TEXT PRIMARY KEY CHECK (id ~ '^qst[a-z0-9]{6}$'),
    text        TEXT NOT NULL,
    source      TEXT NOT NULL CHECK (source IN ('uploaded', 'agent', 'pitch')),
    role        TEXT NOT NULL CHECK (role IN (
        'solutions_architect',
        'software_engineer',
        'engineering_manager',
        'program_manager',
        'product_manager'
    )),
    categories  JSONB NOT NULL DEFAULT '[]'::jsonb,
    company_id  TEXT REFERENCES companies(id) ON DELETE CASCADE,
    added_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX questions_company_id_idx ON questions (company_id);

-- ── sessions ────────────────────────────────────────────────────────────
-- Snapshots (model + prompt) and id lists are JSONB blobs the app reads
-- whole. company_id RESTRICT — anchor company cannot be deleted while a
-- session references it.
CREATE TABLE sessions (
    id                          TEXT PRIMARY KEY CHECK (id ~ '^ses[a-z0-9]{6}$'),
    company_id                  TEXT NOT NULL REFERENCES companies(id) ON DELETE RESTRICT,
    role                        TEXT NOT NULL CHECK (role IN (
        'solutions_architect',
        'software_engineer',
        'engineering_manager',
        'program_manager',
        'product_manager'
    )),
    selected_company_ids        JSONB NOT NULL DEFAULT '[]'::jsonb,
    curated_question_ids        JSONB NOT NULL DEFAULT '[]'::jsonb,
    focus_line                  TEXT NOT NULL DEFAULT '',
    started_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ended_at                    TIMESTAMPTZ,
    status                      TEXT NOT NULL CHECK (status IN ('active', 'ended')),
    model_snapshot              JSONB NOT NULL,
    prompt_snapshot             JSONB NOT NULL,
    voice_critique_enabled      BOOLEAN NOT NULL DEFAULT FALSE,
    current_question_id         TEXT,
    current_question_text       TEXT,
    current_question_audio_path TEXT
);
CREATE INDEX sessions_company_id_idx ON sessions (company_id);
CREATE INDEX sessions_status_started_at_idx ON sessions (status, started_at DESC);

-- ── evaluations ─────────────────────────────────────────────────────────
-- One row per (session, question) attempt-cluster. `attempts` is an array
-- of objects (attempt_n, transcripts, critique scores + narrative, etc.).
CREATE TABLE evaluations (
    id            TEXT PRIMARY KEY CHECK (id ~ '^eva[a-z0-9]{6}$'),
    session_id    TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    company_id    TEXT NOT NULL REFERENCES companies(id) ON DELETE RESTRICT,
    question_id   TEXT NOT NULL REFERENCES questions(id) ON DELETE RESTRICT,
    question_text TEXT NOT NULL,
    attempts      JSONB NOT NULL DEFAULT '[]'::jsonb
);
CREATE INDEX evaluations_session_id_idx ON evaluations (session_id);
CREATE INDEX evaluations_company_id_idx ON evaluations (company_id);
CREATE INDEX evaluations_question_id_idx ON evaluations (question_id);

-- ── summaries ───────────────────────────────────────────────────────────
-- Post-session AI summary. Bullet lists stay JSONB arrays of strings;
-- prose fields are plain TEXT. company_id RESTRICT — historical record.
CREATE TABLE summaries (
    id                   TEXT PRIMARY KEY CHECK (id ~ '^sum[a-z0-9]{6}$'),
    session_id           TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    company_id           TEXT NOT NULL REFERENCES companies(id) ON DELETE RESTRICT,
    narrative            TEXT NOT NULL DEFAULT '',
    strengths            JSONB NOT NULL DEFAULT '[]'::jsonb,
    recurring_weaknesses JSONB NOT NULL DEFAULT '[]'::jsonb,
    blind_spots          JSONB NOT NULL DEFAULT '[]'::jsonb,
    company_fit_signal   TEXT NOT NULL DEFAULT '',
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX summaries_session_id_idx ON summaries (session_id);
CREATE INDEX summaries_company_id_idx ON summaries (company_id);

-- ── stories ─────────────────────────────────────────────────────────────
-- Persistent shell for one competency-tagged STAR+ story. Chat is an
-- append-only array of {role, content, ts} turns — kept inline.
CREATE TABLE stories (
    id                  TEXT PRIMARY KEY CHECK (id ~ '^sto[a-z0-9]{6}$'),
    competency_id       TEXT NOT NULL REFERENCES categories(id) ON DELETE RESTRICT,
    status              TEXT NOT NULL CHECK (status IN ('in_progress', 'complete')),
    mode                TEXT NOT NULL CHECK (mode IN ('strict', 'collaborative')),
    current_version_id  TEXT,
    chat                JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX stories_competency_id_idx ON stories (competency_id);

-- ── story_versions ──────────────────────────────────────────────────────
-- Immutable snapshot per Generate click. UNIQUE(story_id, version_n).
CREATE TABLE story_versions (
    id          TEXT PRIMARY KEY CHECK (id ~ '^stv[a-z0-9]{6}$'),
    story_id    TEXT NOT NULL REFERENCES stories(id) ON DELETE CASCADE,
    version_n   INTEGER NOT NULL,
    body        JSONB NOT NULL,
    spoken      JSONB,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (story_id, version_n)
);
CREATE INDEX story_versions_story_id_idx ON story_versions (story_id);

-- ── pitches (catalog) ───────────────────────────────────────────────────
-- Slug-id, system-seeded catalog. Status/version/chat live here for now;
-- 0004 splits them out into a per-user state table.
CREATE TABLE pitches (
    id                  TEXT PRIMARY KEY,
    question_text       TEXT NOT NULL,
    blurb               TEXT NOT NULL,
    sort_order          INTEGER NOT NULL DEFAULT 0,
    status              TEXT NOT NULL CHECK (status IN ('not_started', 'in_progress', 'locked')),
    current_version_id  TEXT,
    chat                JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ── pitch_versions ──────────────────────────────────────────────────────
CREATE TABLE pitch_versions (
    id          TEXT PRIMARY KEY CHECK (id ~ '^piv[a-z0-9]{6}$'),
    pitch_id    TEXT NOT NULL REFERENCES pitches(id) ON DELETE CASCADE,
    version_n   INTEGER NOT NULL,
    short       TEXT NOT NULL,
    long        TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (pitch_id, version_n)
);
CREATE INDEX pitch_versions_pitch_id_idx ON pitch_versions (pitch_id);

-- ── journal_entries ─────────────────────────────────────────────────────
-- Free-form user journal. Partial index over the active rows for the list
-- view; archived rows are visible only via the data layer.
CREATE TABLE journal_entries (
    id          TEXT PRIMARY KEY CHECK (id ~ '^jnl[a-z0-9]{6}$'),
    title       TEXT NOT NULL,
    body        TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    archived_at TIMESTAMPTZ
);
CREATE INDEX journal_entries_active_updated_at_idx
    ON journal_entries (updated_at DESC)
    WHERE archived_at IS NULL;

-- ── assets ──────────────────────────────────────────────────────────────
-- Uploaded book / resume / other. "Exactly one primary per kind" is
-- enforced as a partial unique index — app code keeps the invariant on
-- toggle, the index is the hard backstop.
CREATE TABLE assets (
    id                TEXT PRIMARY KEY CHECK (id ~ '^ast[a-z0-9]{6}$'),
    name              TEXT NOT NULL,
    kind              TEXT NOT NULL CHECK (kind IN ('book', 'resume', 'other')),
    "primary"         BOOLEAN NOT NULL DEFAULT FALSE,
    original_filename TEXT NOT NULL,
    original_path     TEXT NOT NULL,
    extracted_path    TEXT,
    extraction_status TEXT NOT NULL CHECK (extraction_status IN ('pending', 'ok', 'failed')),
    extraction_error  TEXT,
    uploaded_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE UNIQUE INDEX assets_one_primary_per_kind_idx
    ON assets (kind)
    WHERE "primary" = TRUE;

-- ── prompts ─────────────────────────────────────────────────────────────
-- name is the PK (one row per logical prompt). Allowed names mirror
-- `models::prompt::PROMPT_NAMES` exactly.
CREATE TABLE prompts (
    name               TEXT PRIMARY KEY CHECK (name IN (
        'critique',
        'research',
        'summary',
        'story_chat',
        'story_chat_collaborative',
        'story_summarize',
        'story_refine_open',
        'story_spoken',
        'pitch_chat',
        'pitch_lockin'
    )),
    current_version_id TEXT NOT NULL,
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ── prompt_versions ─────────────────────────────────────────────────────
CREATE TABLE prompt_versions (
    id            TEXT PRIMARY KEY CHECK (id ~ '^prv[a-z0-9]{6}$'),
    prompt_name   TEXT NOT NULL REFERENCES prompts(name) ON DELETE CASCADE,
    body          TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    restored_from TEXT
);
CREATE INDEX prompt_versions_prompt_name_idx ON prompt_versions (prompt_name);

-- ── settings ────────────────────────────────────────────────────────────
-- Singleton row. The id check pins it to the literal 'singleton' so a
-- second row is rejected outright.
CREATE TABLE settings (
    id             TEXT PRIMARY KEY CHECK (id = 'singleton'),
    critique_model TEXT NOT NULL,
    research_model TEXT NOT NULL,
    stt_model      TEXT NOT NULL,
    tts_model      TEXT NOT NULL,
    tts_voice      TEXT NOT NULL,
    tts_language   TEXT NOT NULL DEFAULT '',
    tts_speed      REAL,
    lite_model     TEXT NOT NULL,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
