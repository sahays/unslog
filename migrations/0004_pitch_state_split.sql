-- 0004_pitch_state_split.sql
-- Pitches split into catalog (`pitches`, global, slug-id) + per-user state
-- (`pitch_user_state`, keyed on owner_id + pitch_id). Versions become per-
-- user as well so two users' lock-ins don't collide.
--
-- Idempotent: the data move only runs if the columns we're about to drop
-- still exist (so re-running the migration is a no-op after the columns
-- are gone).

-- ── pitch_user_state ────────────────────────────────────────────────────
CREATE TABLE pitch_user_state (
    owner_id            TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    pitch_id            TEXT NOT NULL REFERENCES pitches(id) ON DELETE CASCADE,
    status              TEXT NOT NULL DEFAULT 'not_started'
                        CHECK (status IN ('not_started', 'in_progress', 'locked')),
    current_version_id  TEXT,
    chat                JSONB NOT NULL DEFAULT '[]'::jsonb,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (owner_id, pitch_id)
);
CREATE INDEX pitch_user_state_pitch_id_idx ON pitch_user_state (pitch_id);

-- ── pitch_versions: add owner_id ────────────────────────────────────────
ALTER TABLE pitch_versions ADD COLUMN owner_id TEXT;
UPDATE pitch_versions SET owner_id = (SELECT id FROM users WHERE is_master = TRUE);
ALTER TABLE pitch_versions ALTER COLUMN owner_id SET NOT NULL;
ALTER TABLE pitch_versions
    ADD CONSTRAINT pitch_versions_owner_id_fkey
    FOREIGN KEY (owner_id) REFERENCES users(id) ON DELETE CASCADE;
CREATE INDEX pitch_versions_owner_id_idx ON pitch_versions (owner_id);

-- ── Backfill pitch_user_state from existing catalog columns ─────────────
-- Only run if the legacy state columns are still on `pitches`. Once they
-- are dropped (below), this block is skipped on re-runs.
DO $$
DECLARE
    has_status BOOLEAN;
BEGIN
    SELECT EXISTS (
        SELECT 1 FROM information_schema.columns
         WHERE table_name = 'pitches' AND column_name = 'status'
    ) INTO has_status;

    IF has_status THEN
        INSERT INTO pitch_user_state (owner_id, pitch_id, status, current_version_id, chat, updated_at)
        SELECT
            (SELECT id FROM users WHERE is_master = TRUE) AS owner_id,
            p.id,
            p.status,
            p.current_version_id,
            p.chat,
            p.updated_at
          FROM pitches p
        ON CONFLICT (owner_id, pitch_id) DO NOTHING;
    END IF;
END
$$;

-- ── Drop legacy state columns from the catalog ──────────────────────────
ALTER TABLE pitches DROP COLUMN IF EXISTS status;
ALTER TABLE pitches DROP COLUMN IF EXISTS current_version_id;
ALTER TABLE pitches DROP COLUMN IF EXISTS chat;
