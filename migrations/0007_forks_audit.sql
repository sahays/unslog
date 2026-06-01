-- 0007_forks_audit.sql
-- Audit trail for the share + fork flow. Each row records that one owner
-- forked a public catalog object (today: companies) into their own space.
-- Source rows stay untouched; this table is forensic only — the new
-- catalog row is created by app-side `services::company_fork::fork`
-- inside a single Postgres transaction so audit-without-fork (or the
-- reverse) is structurally impossible.
--
-- `source_kind` is TEXT + CHECK so we can extend to other forkable kinds
-- (stories, pitches, prompts) later without a column change. `new_id` is
-- not FK-typed — different kinds use different prefixed-id namespaces.

CREATE TABLE forks (
    id          TEXT PRIMARY KEY,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('company')),
    source_id   TEXT NOT NULL,
    new_id      TEXT NOT NULL,
    owner_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT forks_id_format_chk CHECK (id ~ '^frk[a-z0-9]{6}$')
);

CREATE INDEX forks_owner_id_idx ON forks (owner_id);
CREATE INDEX forks_source_idx   ON forks (source_kind, source_id);
