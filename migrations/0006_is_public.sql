-- 0006_is_public.sql
-- Lets any owner publish a company to a public listing. No trigger on
-- who can flip the flag — that policy is enforced (or relaxed) in app
-- code, not the schema.

ALTER TABLE companies
    ADD COLUMN is_public BOOLEAN NOT NULL DEFAULT FALSE;

CREATE INDEX companies_public_updated_at_idx
    ON companies (updated_at DESC)
    WHERE is_public = TRUE;
