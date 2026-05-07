# Behavioral Interview Coach — Vision

## Problem

A personal practice tool for behavioral interviews — answer questions out loud, get specific critique grounded in *Acing Behavioral Interviews, 2e*, and surface mistakes, gaps, and blind spots across multiple sessions.

Existing AI prep tools default to a generic "STAR coach" prompt and don't know company-specific signal. This app is built around **the book as the substrate** and **agentic per-company research** as the role-specific layer.

## Product Shape (v1)

**Live answer coach.** You pick (or get assigned) a behavioral question, answer it by voice, the AI critiques in depth using your book's frameworks plus company-specific signal, you retry the answer, and the AI compares attempts. Reps until you stop.

**Iterative retry as the core loop.** One-shot critique is too thin; the second attempt is where growth happens. Coached dialogue (mid-answer interruptions) is parked for the future "simulator" mode.

## Scoping Model

Everything is scoped per **company**. When you add a company:

1. The research agent pulls a packet (forums, blogs, company site, recent posts, role JD).
2. A question bank is assembled (your uploads + agent-found, shuffled).
3. All sessions, evaluations, and summaries belong to that company.
4. The packet is refreshable on demand.

## AI Brain

- **Critique prompt** = inline-context the relevant chapters of the book (STAR+, pitfalls in Ch 5, role playbook from Ch 8–11, company-culture chapter Ch 12) + the company research packet + the last N session summaries.
- **No RAG.** The book is small enough to fit in context.
- **Models user-switchable in settings** (all four): STT, TTS, critique reasoning, research agent.
- **Defaults**: Gemini 2.5 Pro or Claude Sonnet 4.5 for critique; `openai/gpt-4o-mini-tts` for TTS; Voxtral or Gemini-2.5-with-audio for STT.

## Voice

- STT and TTS both via OpenRouter (single key surface).
- TTS endpoint is first-class on OpenRouter (`/api/v1/audio/speech`).
- STT goes through chat-with-`input_audio` — about 5–10× pure-Whisper cost, but for personal use the dollars are trivial.
- Both paths handle Indian English well.

## Critique UX

- **Text default**, voice toggle, text always visible.
- Long critiques are unreadable as audio — text wins for skimming and review.

## Sessions

- "End Session" button. No timer, no question quota.
- **Per-question evaluation** saved as you go — structured (axis scores, narrative, book-chapter citations).
- **Per-session narrative summary** written at end of session ("strong on conflict stories, recurring weakness: vague metrics; missing 'how did this change your model of the customer'").
- Summaries roll forward into future sessions' critique prompts, so the AI sees patterns over time.

## Recordings & Review

- Audio + transcripts retained per session.
- Stored on disk under `data/recordings/<company_id>/<session_id>/`.
- `data/` is git-ignored.
- Per-session review page: every Q/A, audio playback, transcripts, critiques, and the session summary. Read-only — no edits after the fact, so progress can't be gamed.

## Stack

- **Single Rust crate.**
- **Axum + axum-htmx + Askama + HTMX + Tailwind** for SSR + interactivity.
- **Small JS island** for mic capture (`navigator.mediaDevices.getUserMedia` + `MediaRecorder`) and audio playback in the review page. HTMX/Alpine for everything else.
- **MongoDB** at `mongodb://localhost:27017/behavioral_coach` (running locally in Docker).
- **No auth.** Single-user, local.

## Parked for Later

- **Simulator mode** — AI plays the interviewer end-to-end, follows up mid-answer, full debrief at end. Needs real-time-ish latency, different prompt shape.
- **Cross-session metric trends** — the per-question evaluations support this; only narrative summaries are exposed in v1.
- **Multiple users** — out of scope.
