# Implementation Plan

## Architecture

Single Axum web server, server-rendered Askama templates, HTMX for interactivity, a small JS module for mic capture and playback. Mongo for state, filesystem for audio. All AI providers fronted by OpenRouter through a single client module.

```
Browser
  ├─ HTMX → Askama-rendered HTML fragments
  └─ JS island: MediaRecorder → POST /sessions/:id/answers
                       ↓
                  Axum server
                       ├─ Mongo (state)
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
├─ prompts/                        # critique.md, research.md, summary.md
├─ src/
│  ├─ main.rs
│  ├─ lib.rs
│  ├─ config.rs                    # env loading
│  ├─ error.rs
│  ├─ startup.rs                   # axum app builder
│  ├─ db/                          # mongo client + collection accessors
│  ├─ models/                      # serde structs (Company, Session, Evaluation, Summary, Settings)
│  ├─ routes/
│  │  ├─ mod.rs
│  │  ├─ companies.rs
│  │  ├─ sessions.rs
│  │  ├─ answers.rs
│  │  ├─ review.rs
│  │  ├─ recordings.rs             # streams audio files
│  │  └─ settings.rs
│  ├─ services/
│  │  ├─ openrouter.rs             # one client, four call types
│  │  ├─ critique.rs               # builds prompt from book + packet + history
│  │  ├─ research.rs               # research agent
│  │  ├─ stt.rs
│  │  ├─ tts.rs
│  │  ├─ summary.rs                # end-of-session summarizer
│  │  └─ book.rs                   # extracts/serves chapter text
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
│  └─ settings.html
├─ data/                           # GIT-IGNORED
│  ├─ recordings/
│  └─ book/                        # extracted chapter markdown, cached
└─ tests/
```

## Data Model (Mongo)

Database: `behavioral_coach`.

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

**Indexes:** `evaluations.session_id`, `sessions.company_id`, `summaries.company_id`, unique `companies.name`.

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
| GET  | `/settings`                           | settings form |
| POST | `/settings`                           | save model selections |

## Critique Prompt Assembly

`services::critique::build_prompt` composes, in order:

1. **System message** — role and rubric framing.
2. **Book context blocks** — STAR+ chapter, pitfalls (Ch 5), role-relevant playbook (Ch 8–11), company-culture chapter (Ch 12). Loaded once at startup from `data/book/` (extracted from the PDF on first boot).
3. **Company packet** — brief + role JD + values signal + sample questions.
4. **Prior summaries (last 3)** — surfaces recurring themes for cross-session memory.
5. **Current question + attempt history within this session.**
6. **The new answer transcript.**
7. **Output schema instruction** — JSON with `scores`, `narrative`, `citations`, `improved_vs_prior`.

Use the chosen model's structured-output feature where available; otherwise prompt for JSON and parse defensively.

## Research Agent

OpenRouter chat call with web-search/tool-use enabled. System prompt asks for:
- Company values + how they show up in interviews
- Recent product/strategic context (last ~6 months)
- Role JD and what's specifically evaluated
- Sample behavioral questions reported by candidates (forums, Glassdoor-style sources)

Returns a structured packet stored on the company doc. Idempotent — refresh overwrites.

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
MONGO_URI=mongodb://localhost:27017
MONGO_DB=behavioral_coach
OPENROUTER_API_KEY=
BOOK_PDF_PATH=./assets/book.pdf
RUST_LOG=info,unslog=debug
```

## Build Phases

Order matters — get a working slice end-to-end before polishing.

| Phase | Scope | Est. |
|---|---|---|
| 1 | **Skeleton** — crate, Axum boots, Askama renders, Tailwind builds, Mongo connects, `.env` loads, health-check route | 1d |
| 2 | **Companies + research** — add company form, research agent (call OpenRouter, parse, save), refresh button | 1d |
| 3 | **Question bank** — upload (paste/file), auto-append agent-found at company creation | ½d |
| 4 | **Active session, text-only** — start session, show question, type answer, get critique. Validates prompt assembly + OpenRouter wiring before voice is added | 1d |
| 5 | **Voice in** — mic capture island, audio upload, STT pipeline | 1d |
| 6 | **Voice out** — TTS the question + critique on toggle, audio playback | ½d |
| 7 | **Retry loop** — second-attempt route, comparison-aware critique | ½d |
| 8 | **End session + summary** — end button, summary generation, carry-forward into next session's prompt | ½d |
| 9 | **Per-session review** — read-only Q/A walkthrough with audio playback | ½d |
| 10 | **Settings page** — model pickers, voice/speed selectors | ½d |
| 11 | **Polish** — empty states, error toasts, transcripts visible during recording, keyboard shortcuts | 1d |

Rough total: **6–7 focused days** for a usable v1.

## Risks & Open Questions

1. **Book extraction fidelity.** Pulling clean chapter text from the PDF is its own small project. Plan: use `pdf-extract` or shell out to `pdftotext` once at first boot, save chunked Markdown under `data/book/`, hand-fix any chapters that come out mangled. The user has the source manuscript — if extraction is too lossy, plug that in directly.
2. **OpenRouter STT cost surprise.** Chat-with-audio is ~5–10× Whisper rates. For personal use this is fine; if it ever feels expensive, swap to Groq Whisper behind the same `stt.rs` interface.
3. **Mic permissions in dev.** `getUserMedia` requires HTTPS or localhost. Local dev on `127.0.0.1` is fine; if ever exposed on a LAN, a self-signed cert is needed.
4. **Critique drift / sycophancy.** Without grounding, frontier models get sycophantic. Mitigation: the prompt forces book citations and a numeric specificity score; tighten the system prompt as needed once early outputs are seen.
5. **Simulator mode** later requires real-time turn-taking (~2s round trips). The current sync-POST-per-answer shape won't cut it. Build with that in mind, but don't over-engineer for it now.
