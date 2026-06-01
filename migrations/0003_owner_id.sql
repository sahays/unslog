-- 0003_owner_id.sql
-- Per-user data ownership. Adds owner_id (NOT NULL, FK to users, CASCADE
-- on user delete) to every user-owned table.
--
-- Tables OUT OF SCOPE here:
--   * categories, prompts, prompt_versions, settings — global catalogs.
--   * pitches, pitch_versions — handled in 0004 (catalog/state split).
--   * users, login_attempts — identity layer.
--
-- The data backfill points every existing row at the master user. To make
-- this migration safe on fresh databases (including per-test databases
-- created by `#[sqlx::test]`), we insert a placeholder master row here if
-- one is not already present. `services::master_seed::ensure_master`
-- recognises the placeholder code_hash and force-overwrites it with the
-- real argon2id hash on first boot, so production installs end up with
-- the real hash and nothing else changes.

INSERT INTO users (id, code_hash, code_hint, label, tier, is_master, created_at)
VALUES (
    'usrmaster',
    '$argon2id$v=19$m=19456,t=2,p=1$placeholder$placeholder',
    'plac…er',
    'master',
    'master',
    TRUE,
    NOW()
)
ON CONFLICT (id) DO NOTHING;

-- ── companies ───────────────────────────────────────────────────────────
ALTER TABLE companies ADD COLUMN owner_id TEXT;
UPDATE companies SET owner_id = (SELECT id FROM users WHERE is_master = TRUE);
ALTER TABLE companies ALTER COLUMN owner_id SET NOT NULL;
ALTER TABLE companies
    ADD CONSTRAINT companies_owner_id_fkey
    FOREIGN KEY (owner_id) REFERENCES users(id) ON DELETE CASCADE;
CREATE INDEX companies_owner_id_idx ON companies (owner_id);

-- ── questions ───────────────────────────────────────────────────────────
ALTER TABLE questions ADD COLUMN owner_id TEXT;
UPDATE questions SET owner_id = (SELECT id FROM users WHERE is_master = TRUE);
ALTER TABLE questions ALTER COLUMN owner_id SET NOT NULL;
ALTER TABLE questions
    ADD CONSTRAINT questions_owner_id_fkey
    FOREIGN KEY (owner_id) REFERENCES users(id) ON DELETE CASCADE;
CREATE INDEX questions_owner_id_idx ON questions (owner_id);

-- ── sessions ────────────────────────────────────────────────────────────
ALTER TABLE sessions ADD COLUMN owner_id TEXT;
UPDATE sessions SET owner_id = (SELECT id FROM users WHERE is_master = TRUE);
ALTER TABLE sessions ALTER COLUMN owner_id SET NOT NULL;
ALTER TABLE sessions
    ADD CONSTRAINT sessions_owner_id_fkey
    FOREIGN KEY (owner_id) REFERENCES users(id) ON DELETE CASCADE;
CREATE INDEX sessions_owner_id_idx ON sessions (owner_id);

-- ── evaluations ─────────────────────────────────────────────────────────
ALTER TABLE evaluations ADD COLUMN owner_id TEXT;
UPDATE evaluations SET owner_id = (SELECT id FROM users WHERE is_master = TRUE);
ALTER TABLE evaluations ALTER COLUMN owner_id SET NOT NULL;
ALTER TABLE evaluations
    ADD CONSTRAINT evaluations_owner_id_fkey
    FOREIGN KEY (owner_id) REFERENCES users(id) ON DELETE CASCADE;
CREATE INDEX evaluations_owner_id_idx ON evaluations (owner_id);

-- ── summaries ───────────────────────────────────────────────────────────
ALTER TABLE summaries ADD COLUMN owner_id TEXT;
UPDATE summaries SET owner_id = (SELECT id FROM users WHERE is_master = TRUE);
ALTER TABLE summaries ALTER COLUMN owner_id SET NOT NULL;
ALTER TABLE summaries
    ADD CONSTRAINT summaries_owner_id_fkey
    FOREIGN KEY (owner_id) REFERENCES users(id) ON DELETE CASCADE;
CREATE INDEX summaries_owner_id_idx ON summaries (owner_id);

-- ── stories ─────────────────────────────────────────────────────────────
ALTER TABLE stories ADD COLUMN owner_id TEXT;
UPDATE stories SET owner_id = (SELECT id FROM users WHERE is_master = TRUE);
ALTER TABLE stories ALTER COLUMN owner_id SET NOT NULL;
ALTER TABLE stories
    ADD CONSTRAINT stories_owner_id_fkey
    FOREIGN KEY (owner_id) REFERENCES users(id) ON DELETE CASCADE;
CREATE INDEX stories_owner_id_idx ON stories (owner_id);

-- ── story_versions ──────────────────────────────────────────────────────
ALTER TABLE story_versions ADD COLUMN owner_id TEXT;
UPDATE story_versions SET owner_id = (SELECT id FROM users WHERE is_master = TRUE);
ALTER TABLE story_versions ALTER COLUMN owner_id SET NOT NULL;
ALTER TABLE story_versions
    ADD CONSTRAINT story_versions_owner_id_fkey
    FOREIGN KEY (owner_id) REFERENCES users(id) ON DELETE CASCADE;
CREATE INDEX story_versions_owner_id_idx ON story_versions (owner_id);

-- ── journal_entries ─────────────────────────────────────────────────────
-- Recreate the active-rows partial index to include owner_id so the per-user
-- list view can hit it directly.
ALTER TABLE journal_entries ADD COLUMN owner_id TEXT;
UPDATE journal_entries SET owner_id = (SELECT id FROM users WHERE is_master = TRUE);
ALTER TABLE journal_entries ALTER COLUMN owner_id SET NOT NULL;
ALTER TABLE journal_entries
    ADD CONSTRAINT journal_entries_owner_id_fkey
    FOREIGN KEY (owner_id) REFERENCES users(id) ON DELETE CASCADE;
CREATE INDEX journal_entries_owner_id_idx ON journal_entries (owner_id);
DROP INDEX IF EXISTS journal_entries_active_updated_at_idx;
CREATE INDEX journal_entries_owner_active_updated_at_idx
    ON journal_entries (owner_id, updated_at DESC)
    WHERE archived_at IS NULL;

-- ── assets ──────────────────────────────────────────────────────────────
-- Rescope the "one primary per kind" invariant to be per-owner. Also keep
-- the `book` asset master-only: the critique prompt inlines the book and
-- the book is global content, not per-user.
ALTER TABLE assets ADD COLUMN owner_id TEXT;
UPDATE assets SET owner_id = (SELECT id FROM users WHERE is_master = TRUE);
ALTER TABLE assets ALTER COLUMN owner_id SET NOT NULL;
ALTER TABLE assets
    ADD CONSTRAINT assets_owner_id_fkey
    FOREIGN KEY (owner_id) REFERENCES users(id) ON DELETE CASCADE;
CREATE INDEX assets_owner_id_idx ON assets (owner_id);
DROP INDEX IF EXISTS assets_one_primary_per_kind_idx;
CREATE UNIQUE INDEX assets_one_primary_per_owner_kind_uidx
    ON assets (owner_id, kind)
    WHERE "primary" = TRUE;

-- Trigger: kind='book' rows must be owned by the master user.
CREATE OR REPLACE FUNCTION assets_book_must_be_master_fn()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.kind = 'book' THEN
        IF NOT EXISTS (
            SELECT 1 FROM users
             WHERE id = NEW.owner_id AND is_master = TRUE
        ) THEN
            RAISE EXCEPTION
                'asset kind=book must be owned by the master user (got owner_id=%)',
                NEW.owner_id;
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER assets_book_must_be_master_trg
    BEFORE INSERT OR UPDATE OF kind, owner_id ON assets
    FOR EACH ROW
    EXECUTE FUNCTION assets_book_must_be_master_fn();
