-- 0002_users.sql
-- Multi-user identity. Adds the users table (master + invited) and a
-- login_attempts audit log used for rate-limiting / abuse detection.
--
-- Master user has the reserved literal id `usrmaster` so the rest of the
-- schema (0003+) can FK to it deterministically during backfill, even on
-- fresh installs where ids would otherwise be CSPRNG-generated.

CREATE TABLE users (
    id            TEXT PRIMARY KEY CHECK (id = 'usrmaster' OR id ~ '^usr[a-z0-9]{6}$'),
    -- Argon2 hash of the invite code. Never store the raw code.
    code_hash     TEXT NOT NULL,
    -- Short user-visible hint shown next to the invite code field on login.
    code_hint     TEXT NOT NULL DEFAULT '',
    label         TEXT NOT NULL,
    tier          TEXT NOT NULL CHECK (tier IN ('pro', 'pro_max', 'master')),
    is_master     BOOLEAN NOT NULL DEFAULT FALSE,
    activated_at  TIMESTAMPTZ,
    expires_at    TIMESTAMPTZ,
    last_seen_at  TIMESTAMPTZ,
    revoked_at    TIMESTAMPTZ,
    invited_by    TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Master must have tier='master' AND never expire.
    CONSTRAINT users_master_consistency_chk CHECK (
        NOT is_master OR (tier = 'master' AND expires_at IS NULL)
    )
);
CREATE UNIQUE INDEX users_code_hash_uidx ON users (code_hash);
-- Exactly one master, ever.
CREATE UNIQUE INDEX users_single_master_uidx ON users ((TRUE)) WHERE is_master;
CREATE INDEX users_tier_idx ON users (tier);
CREATE INDEX users_expires_at_idx ON users (expires_at);
CREATE INDEX users_invited_by_idx ON users (invited_by);

-- login_attempts: rolling audit log. Only the first 4 chars of the code
-- are recorded — enough for forensic grouping, never enough to replay.
CREATE TABLE login_attempts (
    id           BIGSERIAL PRIMARY KEY,
    ip           INET,
    code_prefix  TEXT NOT NULL,
    success      BOOLEAN NOT NULL,
    attempted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT login_attempts_prefix_len_chk CHECK (char_length(code_prefix) <= 4)
);
CREATE INDEX login_attempts_ip_attempted_at_idx ON login_attempts (ip, attempted_at DESC);
