# Implementation Plan

## Architecture

Single Axum web server, server-rendered Askama templates, HTMX for interactivity, a small JS module for mic capture and playback. Postgres for state (via sqlx), filesystem for audio. All AI providers fronted by OpenRouter through a single client module.

```
Browser
  ├─ HTMX → Askama-rendered HTML fragments
  └─ JS island: MediaRecorder → POST /sessions/:id/answers
                       ↓
                  Axum server
                       ├─ Postgres (state, via sqlx)
                       ├─ data/recordings/ (audio files)
                       └─ OpenRouter
                            ├─ STT (chat with input_audio)
                            ├─ TTS (/audio/speech)
                            ├─ Critique (text chat)
                            └─ Research agent (text chat + web tools)
```

## Project Layout

```
unslog/
├─ Cargo.toml
├─ build.rs                        # tailwind build hook
├─ .env.example
├─ .gitignore
├─ docs/
├─ prompts/                        # default seed prompts, embedded via include_str!
├─ src/
│  ├─ main.rs
│  ├─ lib.rs
│  ├─ config.rs                    # env loading
│  ├─ error.rs
│  ├─ startup.rs                   # axum app builder
│  ├─ services/db.rs               # postgres pool + sqlx migrations + helpers
│  ├─ models/                      # serde structs (Company, Session, Evaluation, Summary, Settings, Asset, Prompt, PromptVersion)
│  ├─ routes/
│  │  ├─ mod.rs
│  │  ├─ companies.rs
│  │  ├─ sessions.rs
│  │  ├─ answers.rs
│  │  ├─ review.rs
│  │  ├─ recordings.rs             # streams audio files
│  │  ├─ assets.rs                 # upload/list/preview the book and any other works
│  │  ├─ prompts.rs                # edit + version history for system prompts
│  │  └─ settings.rs
│  ├─ services/
│  │  ├─ openrouter.rs             # one client, four call types
│  │  ├─ critique.rs               # builds prompt from primary asset + packet + history
│  │  ├─ research.rs               # research agent
│  │  ├─ stt.rs
│  │  ├─ tts.rs
│  │  ├─ summary.rs                # end-of-session summarizer
│  │  ├─ assets.rs                 # PDF text extraction; serves extracted text for prompt assembly
│  │  └─ prompt_store.rs           # load current or a specific version of a named prompt
│  ├─ recordings.rs                # disk paths, write/read helpers
│  └─ logging.rs
├─ static/
│  ├─ css/                         # tailwind output
│  └─ js/
│     ├─ recorder.js               # MediaRecorder island
│     └─ player.js                 # audio playback in review
├─ templates/
│  ├─ base.html
│  ├─ companies/
│  ├─ sessions/
│  ├─ review/
│  ├─ assets/
│  ├─ prompts/                     # editor + version history
│  └─ settings.html
├─ data/                           # GIT-IGNORED
│  ├─ recordings/
│  └─ assets/
│     ├─ originals/                # uploaded PDFs (and other source files)
│     └─ extracted/                # extracted Markdown per asset
└─ tests/
```

## Data Model (Postgres)

Database: `unslog` (project-scoped container `unslog-pg`). Schema lives in
`migrations/`. The document-style summaries below describe the conceptual
shape; the actual schema is normalized into Postgres tables with JSONB
for the embedded sub-documents.

### `companies`
```
{
  _id, name, role,
  created_at, updated_at,
  research_packet: {
    summary,                      # synthesized brief
    sources: [{ url, title, fetched_at, snippet }],
    role_jd,
    values_signal,
    sample_questions: [String],
    research_prompt_version_id,   # which research prompt produced this packet
    last_refreshed_at
  }
}
```

### `question_banks`
```
{
  _id, company_id,
  questions: [{ id, text, source: "uploaded" | "agent", added_at }]
}
```

### `sessions`
```
{
  _id, company_id, started_at, ended_at,
  status: "active" | "ended",
  model_snapshot: { stt, tts, critique, research },   # what was used
  prompt_snapshot: {                                  # frozen at session start
    critique: prompt_version_id,
    summary: prompt_version_id
  },
  voice_critique_enabled: bool
}
```

### `evaluations` — one document per question per session
```
{
  _id, session_id, company_id, question_id, question_text,
  attempts: [
    {
      attempt_n,
      answer_audio_path: "data/recordings/<co>/<sess>/q03_answer_v1.webm",
      answer_transcript,
      critique: {
        scores: { specificity, role_clarity, star_plus_structure, pitfalls_avoided, company_fit },
        narrative,
        citations: [{ chapter, section, quote }],
        improved_vs_prior            # only on attempt_n > 1
      },
      critique_audio_path: Optional,
      created_at
    }
  ]
}
```

### `summaries`
```
{
  _id, session_id, company_id,
  narrative,
  strengths: [String],
  recurring_weaknesses: [String],
  blind_spots: [String],
  company_fit_signal,
  created_at
}
```

### `assets`
```
{
  _id, name,                       # e.g. "Acing Behavioral Interviews 2e"
  kind: "book" | "other",
  primary: bool,                   # exactly one asset is marked primary at any time
  original_filename, original_path,    # data/assets/originals/<id>.pdf
  extracted_path,                      # data/assets/extracted/<id>.md
  extraction_status: "pending" | "ok" | "failed",
  extraction_error: Optional,
  uploaded_at
}
```

### `prompts`
```
{
  _id: "critique" | "research" | "summary",
  current_version_id,
  updated_at
}
```

### `prompt_versions`
```
{
  _id, prompt_name,                # FK to prompts._id
  body,
  created_at,
  restored_from: Optional          # set if this version was created via "Restore version X"
}
```

### `settings` — singleton, `_id: "global"`
```
{
  models: {
    stt: { id, options },
    tts: { id, voice, speed, options },
    critique: { id, options },
    research: { id, options }
  }
  # API key stays in .env; never persisted here
}
```

**Indexes:** `evaluations.session_id`, `sessions.company_id`, `summaries.company_id`, unique `companies.name`, `prompt_versions.prompt_name`, `assets.primary`. The "exactly one primary asset" invariant is enforced in app code (toggling primary unsets others) rather than via a partial unique index.

## HTTP Routes

| Method | Path | Purpose |
|---|---|---|
| GET  | `/`                                   | redirect to `/companies` |
| GET  | `/companies`                          | list + add form |
| POST | `/companies`                          | create + kick off research agent |
| POST | `/companies/:id/refresh-packet`       | re-run research agent |
| GET  | `/companies/:id`                      | dashboard: question bank, sessions, summaries |
| POST | `/companies/:id/questions`            | upload (paste text or file) |
| POST | `/companies/:id/sessions`             | start a session |
| GET  | `/sessions/:id`                       | active session UI |
| POST | `/sessions/:id/next-question`         | pick next, TTS the question, return HTML + audio URL |
| POST | `/sessions/:id/answers`               | multipart audio → STT → critique → return HTML fragment |
| POST | `/sessions/:id/answers/:eval_id/retry`| second attempt, comparison-aware critique |
| POST | `/sessions/:id/end`                   | run summary, mark ended, redirect to review |
| GET  | `/sessions/:id/review`                | per-session review (read-only) |
| GET  | `/recordings/*path`                   | stream audio file (validate path inside `data/recordings/`) |
| GET  | `/assets`                             | list + upload form |
| POST | `/assets`                             | multipart upload, kicks off extraction |
| POST | `/assets/:id/primary`                 | mark this asset primary (unsets the previous one) |
| POST | `/assets/:id/reextract`               | re-run text extraction |
| POST | `/assets/:id/delete`                  | delete asset (forbid if primary and only one exists) |
| GET  | `/assets/:id/preview`                 | view extracted Markdown |
| GET  | `/prompts`                            | list of editable prompts (critique, research, summary) |
| GET  | `/prompts/:name`                      | current text + textarea editor |
| POST | `/prompts/:name`                      | save → creates a new version, sets it current |
| GET  | `/prompts/:name/history`              | chronological list of versions |
| GET  | `/prompts/:name/versions/:version_id` | view a specific version |
| POST | `/prompts/:name/restore/:version_id`  | create a new version from an old one, set current |
| GET  | `/settings`                           | settings form |
| POST | `/settings`                           | save model selections |

## Critique Prompt Assembly

`services::critique::build_prompt` composes, in order:

1. **System message** — loaded from the `critique` prompt-version snapshotted on this session (`session.prompt_snapshot.critique`). Iterating on the prompt later doesn't retroactively change historical critiques.
2. **Book context blocks** — extracted text from the **primary asset** (`assets.primary == true`), loaded from `data/assets/extracted/<asset_id>.md`. The system prompt names which sections to emphasise: STAR+, pitfalls (Ch 5), role-relevant playbook (Ch 8–11), company-culture chapter (Ch 12).
3. **Company packet** — brief + role JD + values signal + sample questions.
4. **Prior summaries (last 3)** — surfaces recurring themes for cross-session memory.
5. **Current question + attempt history within this session.**
6. **The new answer transcript.**
7. **Output schema instruction** — JSON with `scores`, `narrative`, `citations`, `improved_vs_prior`.

Use the chosen model's structured-output feature where available; otherwise prompt for JSON and parse defensively.

## Research Agent

OpenRouter chat call with web-search/tool-use enabled. System prompt is loaded from the **current** `research` prompt version at company-creation or refresh time, and the version ID is recorded on the resulting packet (`research_packet.research_prompt_version_id`) so packet provenance is preserved. The default prompt asks for:

- Company values + how they show up in interviews
- Recent product/strategic context (last ~6 months)
- Role JD and what's specifically evaluated
- Sample behavioral questions reported by candidates (forums, Glassdoor-style sources)

Returns a structured packet stored on the company doc. Idempotent — refresh overwrites.

## Assets

The book and any future works are uploaded through the app — there is no hardcoded book path.

**Upload flow:** multipart POST to `/assets` → write the original to `data/assets/originals/<asset_id>.<ext>` → enqueue a synchronous extraction job (small enough to run inline) → write Markdown to `data/assets/extracted/<asset_id>.md` → set `extraction_status: "ok"`. PDF extraction uses `pdf-extract` first; if that produces obviously broken output, the fallback is shelling out to `pdftotext` (poppler).

**Primary asset:** exactly one asset is `primary: true` at any time. `POST /assets/:id/primary` does the swap atomically. The critique pipeline always inlines the primary asset's extracted text.

**Re-extraction:** `POST /assets/:id/reextract` re-runs extraction (e.g., after upgrading the PDF parser). The extracted Markdown file is also editable on disk — useful for hand-fixing mangled chapters; the app reads it fresh on each critique build.

## Prompts

Three prompts are user-editable: `critique`, `research`, `summary`. (STT's "transcribe verbatim" instruction is operational and not in the editor.)

**Seeding:** defaults live at `prompts/critique.md`, `prompts/research.md`, `prompts/summary.md` and are embedded in the binary via `include_str!`. On first boot, if a `prompts` row doesn't exist for a given name, an initial `prompt_versions` row is created from the embedded default and pointed at by `prompts.<name>.current_version_id`.

**Editing:** `POST /prompts/:name` does **not** mutate the existing version. It always creates a new `prompt_versions` row, then updates `prompts.<name>.current_version_id`. There is no overwrite path. "Restore version X" is the same operation, just seeded from an older version's body.

**Snapshotting at session start:** `POST /companies/:id/sessions` reads the current version IDs for `critique` and `summary` and copies them into `session.prompt_snapshot`. The critique and summary services always resolve prompt text via the snapshot, never via "current." Research prompts snapshot at packet-creation time (see Research Agent).

**History UI:** `GET /prompts/:name/history` lists versions newest-first with timestamp and "restored from" badge if applicable. Clicking a version shows its full body. Diff-between-versions is parked.

## STT / TTS

**`stt.rs`** — read audio file → base64 → POST to `/api/v1/chat/completions` with `input_audio` content + a "transcribe verbatim" instruction → strip non-transcript text from the response.

**`tts.rs`** — POST text to `/api/v1/audio/speech` with the configured model, voice, speed → write MP3 to disk → return relative path.

Both consult `settings.models` at call time. The session records the model snapshot so historical evaluations remain consistent.

## Mic Capture (JS island)

`static/js/recorder.js` — small, vanilla JS:
- "Start" requests mic permission, opens `MediaRecorder` with `audio/webm`.
- "Stop" gets the blob, POSTs as multipart to `/sessions/:id/answers` (or retry endpoint).
- Server returns HTMX-swap-friendly HTML for the critique panel.

Alpine.js for the recording-state UI (idle / recording / uploading / done).

## Settings

GET pulls OpenRouter `/models` (cached 1h), filters by capability, renders four `<select>`s. POST writes to the `settings` doc. Voice/speed depends on the chosen TTS model — re-render that section via HTMX on TTS-model change.

API key lives in `.env` only. The settings page shows "key configured ✓" / "missing" — never the value.

## .gitignore

```
/target
/node_modules
/data
.env
.env.local
.env.*.local
!.env.example
*.db
.DS_Store
```

## .env.example

```
PORT=3000
HOST=127.0.0.1
DATABASE_URL=postgres://unslog:unslog@localhost:5432/unslog
SQLX_OFFLINE=true
OPENROUTER_API_KEY=
RUST_LOG=info,unslog=debug
```

## Build Phases

Order matters — get a working slice end-to-end before polishing.

| Phase | Scope | Est. |
|---|---|---|
| 1 | **Skeleton** — crate, Axum boots, Askama renders, Tailwind builds, Postgres connects + sqlx migrations apply, `.env` loads, health-check route | 1d |
| 2 | **Assets** — upload PDF (multipart), `pdf-extract` pipeline (with `pdftotext` fallback), save originals + extracted Markdown, mark primary, list/preview UI, re-extract button | 1d |
| 3 | **Prompts** — embed defaults via `include_str!`, seed-on-boot, edit form (saves a new version), history view, restore | 1d |
| 4 | **Companies + research** — add company form, research agent (uses current research prompt; records version on packet), refresh button | 1d |
| 5 | **Question bank** — upload (paste/file), auto-append agent-found at company creation | ½d |
| 6 | **Active session, text-only** — start session (snapshots model + prompt versions), show question, type answer, get critique. Validates the full prompt assembly using the primary asset | 1d |
| 7 | **Voice in** — mic capture island, audio upload, STT pipeline | 1d |
| 8 | **Voice out** — TTS the question + critique on toggle, audio playback | ½d |
| 9 | **Retry loop** — second-attempt route, comparison-aware critique | ½d |
| 10 | **End session + summary** — end button, summary generation (uses snapshot summary prompt), carry-forward into future sessions | ½d |
| 11 | **Per-session review** — read-only Q/A walkthrough with audio playback; surfaces which model + prompt versions were used | ½d |
| 12 | **Settings page** — model pickers, voice/speed selectors | ½d |
| 13 | **Polish** — empty states, error toasts, transcripts visible during recording, keyboard shortcuts | 1d |

Rough total: **8–9 focused days** for a usable v1.

## Risks & Open Questions

1. **PDF extraction fidelity.** Asset extraction runs synchronously on upload via `pdf-extract`, with `pdftotext` (poppler) as fallback. Output is Markdown under `data/assets/extracted/`. If a particular asset comes out mangled, the upload UI exposes a "re-extract" button, and the extracted Markdown is editable on disk for hand-fixes. For the book specifically, the source manuscript is available if PDF extraction is unsatisfactory.
2. **OpenRouter STT cost surprise.** Chat-with-audio is ~5–10× Whisper rates. For personal use this is fine; if it ever feels expensive, swap to Groq Whisper behind the same `stt.rs` interface.
3. **Mic permissions in dev.** `getUserMedia` requires HTTPS or localhost. Local dev on `127.0.0.1` is fine; if ever exposed on a LAN, a self-signed cert is needed.
4. **Critique drift / sycophancy.** Without grounding, frontier models get sycophantic. Mitigation: the prompt forces book citations and a numeric specificity score; tighten the system prompt as needed once early outputs are seen.
5. **Simulator mode** later requires real-time turn-taking (~2s round trips). The current sync-POST-per-answer shape won't cut it. Build with that in mind, but don't over-engineer for it now.
