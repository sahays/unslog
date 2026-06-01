-- 0005_request_log.sql
-- Per-request audit + cost accounting. BRIN on ts because the table is
-- append-only and time-correlated (cheap, tiny vs btree).

CREATE TABLE request_log (
    id                   BIGSERIAL PRIMARY KEY,
    user_id              TEXT REFERENCES users(id) ON DELETE CASCADE,
    ts                   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    method               TEXT NOT NULL,
    path                 TEXT NOT NULL,
    status               SMALLINT NOT NULL CHECK (status BETWEEN 100 AND 599),
    duration_ms          INTEGER,
    kind                 TEXT NOT NULL CHECK (kind IN (
        'page_nav', 'llm', 'tts', 'stt', 'upload', 'other'
    )),
    cost_units           INTEGER NOT NULL DEFAULT 0,
    llm_input_tokens     INTEGER,
    llm_output_tokens    INTEGER,
    llm_cost_usd_micros  BIGINT,
    meta                 JSONB NOT NULL DEFAULT '{}'::jsonb
);
CREATE INDEX request_log_user_id_ts_idx ON request_log (user_id, ts DESC);
CREATE INDEX request_log_ts_brin_idx ON request_log USING BRIN (ts);
