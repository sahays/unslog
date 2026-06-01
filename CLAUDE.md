# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`unslog` is a personal-use behavioral-interview coach: a single-binary Axum web app that drives mic-captured answers through STT → critique → TTS using OpenRouter as the LLM gateway. Server-rendered Askama templates + HTMX; small vanilla-JS islands for mic capture; Postgres for state; the filesystem for audio. There is no auth — it is single-user, local-first.

See `docs/plan.md` for the full architecture / data-model / route table (kept up to date with the implementation).

## Coding instructions
 - DRY: Reuse and generalize, do not duplicate e.g. Reuse Rust functions, macros (forms.html, ui.html), shared validators, htmx_error helper, etc. Respect what came before but verify.
 - Composition: Create simple reusable components, functions, structs and compose them to build complex features
 - No brute force: Ensure most optimal data structure and algorithm use. Never use brute force techniques to develop a feature
 - Readability: Ensure higher level abstractions and functions read like pseudocode. Routes should call services and so on with no direct Rust or library primitives
 - Refactor: While developing a new feature or fixing bugs. No function should be longer than 15-20 lines of code (pre-formatting) and no Rust/HTML/TS file longer than 300 lines
 - Naming guidelines: Use language specific standard/universal naming guidelines
 - Frontend: Always write (or write code that generate) semantic HTML
 - Logging: Ensure proper logging that answers who, what, when, why, where, and how. Use semantically correct level of logging
 - Security: Ensure OWASP-level secure coding practices https://owasp.org/www-project-secure-coding-practices-quick-reference-guide/assets/docs/OWASP_SCP_Quick_Reference_Guide_v21.pdf 
 - Testing: Ensure unit tests (functional and security with edge cases) with mocks (network, database, remote APIs, etc.) for all critical paths. Favor quality over quantity.

## Commands

```bash
# Dev server (preferred — runs cargo build, restarts on change)
cargo watch -x run               # the user typically has this running already
cargo run                        # one-shot

# Strict pre-deploy gate (format → clippy -D warnings → check → test)
./scripts/pre-deploy.sh

# Faster inner loop
cargo check
cargo clippy --all-targets -- -D warnings -A clippy::needless_pass_by_value -A clippy::redundant_closure_for_method_calls
cargo test --quiet
cargo test <name_substring>      # single test (filename or fn name)

# Release build
./scripts/build.sh               # cargo build --release

# Pre-flight check (cargo / npm / docker / postgres / sqlx-cli / pdftotext / .env / OPENROUTER_API_KEY)
./scripts/check-deps.sh
./scripts/dev-up.sh              # runs check-deps then `cargo run`

# One-shot Mongo → Postgres copy (run once after Postgres is up)
cargo run --bin import_from_mongo -- --force
```

The user runs `cargo watch -x run` themselves — don't spawn a parallel `cargo run`. Use `cargo check` / `cargo clippy` / `cargo test` for verification, and rely on their browser to exercise the UI.

`cargo test` requires a reachable Postgres on `DATABASE_URL` (the cross-tenant integration suite under `tests/cross_tenant/` is `#[sqlx::test]` — it spins up a per-test database against the live server). Start one with `docker run -d -p 5433:5432 -e POSTGRES_USER=unslog -e POSTGRES_PASSWORD=unslog -e POSTGRES_DB=unslog --name unslog-pg postgres:17`. Migration 0003 self-bootstraps a placeholder master user; the real argon2id hash is written by `services::master_seed::ensure_master` on first boot.

## Required external services

- **Postgres** on `postgres://unslog:unslog@localhost:5432/unslog` (project-scoped container `unslog-pg`). Schema lives in `migrations/`; `sqlx::migrate!()` runs on first boot.
- **OpenRouter API key** in `.env` as `OPENROUTER_API_KEY`. Without it, the settings page shows "missing" and any STT/TTS/chat call returns `AppError::OpenRouterNotConfigured` (503). Other features still work.
- **`pdftotext` (poppler)** is an *optional* fallback for PDF extraction. `pdf-extract` is tried first.
- **`npx` + `@tailwindcss/cli`** — `build.rs` shells out to `npx @tailwindcss/cli` on every cargo build to regenerate `static/css/app.css` from `static/css/input.css`. Missing npx becomes a `cargo:warning`, not a hard failure.

MongoDB is required only if you intend to run `cargo run --bin import_from_mongo -- --force` once to migrate legacy data; the live app no longer connects.

## Architecture

### Request flow

```
Browser ──HTMX──▶ Axum route ──▶ service module ──▶ Postgres / OpenRouter / FS
                       │
                       └─▶ Askama template ──HTML fragment──▶ HTMX swap
```

Three boundary categories, kept separate:

1. **`routes/*`** — HTTP handlers. Parse, validate, call services, render templates. Each top-level resource has its own file/module; `routes/mod.rs` merges them in `router()`. The `sessions` and `stories` resources are large enough to be subdirectories (`routes/sessions/{lifecycle,answer}.rs`, `routes/stories/{landing,show,chat}.rs`).
2. **`services/*`** — business logic, no HTTP types. `openrouter.rs` is the one outbound client (chat / STT / TTS / `/models`); everything else (`critique`, `summary`, `research`, `categorize`, `curator`, `questions`, `stt`, `tts`, `assets`, `*_store`) composes it with Postgres + sqlx. `*_store` modules own a Postgres table.
3. **`models/*`** — serde structs that map 1:1 to Postgres tables. Each model file owns any pure logic on the struct. `models/mod.rs` re-exports the public types. (A handful of legacy `_id` serde renames + `COLLECTION` consts + `datetime_compat` are retained for the one-shot `import_from_mongo` binary and are removed in a follow-up cleanup commit.)

`startup::run` builds `AppState { config, pool, http, openrouter, models_cache, book_cache, … }` and wires it into a single `Router` with a 50 MB body cap (for audio uploads + the book PDF) and a request-context middleware that issues an `x-request-id`.

### Mock seams

The LLM client and several service dependency layers are abstracted behind `#[cfg_attr(test, mockall::automock)]` traits so handlers can be tested without network or DB I/O:

- `services::openrouter::LlmClient` — held as `Arc<dyn LlmClient>` on `AppState`. Production is `OpenRouter`; tests inject `MockLlmClient`.
- `services::critique::CritiqueDeps`, plus equivalent traits in `summary`, `evaluations`, etc. — each abstracts the (DB + cache) reads a service needs so tests can stub them.

Keep this pattern when adding a new service that touches Postgres + LLM: define a `*Deps` trait, ship a `*Ctx<'a>` production impl, decorate with `#[cfg_attr(test, mockall::automock)]`, and have route handlers depend on the trait.

### Prompts: snapshot-on-use

Prompts (`critique`, `research`, `summary`, `story_*`) are user-editable. Each save writes a new `prompt_versions` row and updates `prompts.<name>.current_version_id` — there is no in-place edit and no overwrite. **Sessions snapshot the current version IDs at start time** (`session.prompt_snapshot`), and the critique/summary services resolve prompt text via that snapshot, never via "current." Research prompts snapshot at packet-creation time. Defaults live in `prompts/*.md` and are embedded via `include_str!` and seeded on first boot by `services::prompt_store::seed_defaults`.

When changing prompt text or shape: edit `prompts/*.md`, not the database — the seed code only inserts when the row is missing, so existing installs need a manual new version through the UI (or wipe the `prompt_versions` collection for that prompt name).

### Assets: one is primary

`POST /assets` stores the upload at `data/assets/originals/<id>.<ext>` and extracts to `data/assets/extracted/<id>.md` (pdf-extract → pdftotext fallback). Exactly one asset has `primary: true`; the invariant is enforced in app code on toggle, not by a partial unique index. The critique prompt always inlines the primary asset's extracted text, fetched through `services::assets::BookCache`. The extracted Markdown file is editable on disk — re-read on every critique build.

### Audio

`static/js/recorder.js` is a small vanilla-JS island using `MediaRecorder` (audio/webm). Answer audio is POSTed multipart to `/sessions/:id/answers`; question + critique audio is TTS-generated server-side and served by `routes::recordings::stream` which validates the requested path is inside `data/recordings/` before reading. Filename layout is documented at the top of `src/recordings.rs`.

### LLM output → HTML

LLM output is rendered through `filters::safe_markdown` (exposed as the `safe_markdown` Askama filter, then `|safe`). It deliberately disables `Options::ENABLE_HTML` *and* runs the result through `ammonia` — both passes are necessary because the input is untrusted. Don't replace this with the built-in `|markdown` filter on LLM output.

### Errors

`AppError` (in `src/error.rs`) is the single error type for routes. It implements `IntoResponse`: HTMX requests get a minimal fragment, full-page requests get `templates/errors/error.html`. `OpenRouterNotConfigured` is its own variant and renders as 503 with a "set OPENROUTER_API_KEY" message — return this rather than panicking when a key is missing.

## Conventions

- **Don't write to public repos with personal context.** Scrub absolute personal paths, private-repo names, and personal identifiers before committing if this repo ever goes public.
- **Single-binary, no microservices.** When tempted to split a service out, just add another module under `src/services/`.
- **Server-rendered HTML is the default.** Add a JSON endpoint only if there's a JS island that needs it; today only `recorder.js` does, and it posts multipart and consumes HTML fragments.
- **Templates live in `templates/<resource>/`** with shared macros in `templates/macros/` and the layout in `base.html`. `askama.toml` registers `templates/` as the only template root.
- **Filesystem layout:** code that constructs paths under `data/` goes through `src/recordings.rs` or `services::assets`. Don't sprinkle `format!("data/...")` in handlers.
- **Logging:** `tracing` with `event = "domain.verb"` and structured fields (`session_id = %id`, `company_id = %company.id`). The subscriber writes to stdout *and* a daily-rotating file at `data/logs/unslog.log` (paths configurable via `DATA_DIR` / `LOG_DIR`). Set `LOG_FORMAT=json` to emit JSON for log shippers.
