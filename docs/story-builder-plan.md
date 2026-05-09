# Story Builder — Implementation Plan

## Goal

A separate prep tool — `/stories` — where a smart model probes the candidate's past
experiences against a chosen competency, drives a STAR+/CLEAR/ADAPT chat until coverage
is mutually agreed, and on user demand generates a scannable bullet-form story. Stories
support refine (continue chat → next version) and discard. Chat history is preserved as
the source of truth; bullets are the index.

Distinct from `/sessions` (live mock interviews + critique) and `/practice`
(cross-company live runs).

## Decisions (settled in /clarity)

| Decision | Choice |
|---|---|
| Engine | Single smart-model system prompt with STAR+ as the spine, CLEAR/ADAPT as embedded probing styles |
| Smart model | `settings.critique_model` (reused) — no new setting in v1 |
| Entry | `/stories` landing → competency grid (existing `Category` collection); each tile shows in-progress + complete counts |
| Story-per-competency | Multiple, no cap |
| Lifecycle | Chat ⇄ user clicks **Generate** → version locked. Refine = continue chat → next Generate = vN+1 |
| Coverage handshake | AI verbally proposes "ready to summarize?" when it judges coverage met; user clicks **Generate** to lock in. Premature Generate is allowed (not blocked) |
| Output shape | STAR+ bullets per section (Situation / Task / Action / Result / Reflection). No narrative prose; chat is the deep source |
| Refine startup | **Option X** — AI reads the current version's bullets, picks the thinnest section, opens with one targeted probe |
| Discard | Cascade-delete Story + all StoryVersions + chat |
| Out of scope (v1) | Voice/TTS in chat, live-interview integration, context-window compaction for long chats |

## Data model

### `stories` collection

```rust
pub struct Story {
    #[serde(rename = "_id")]
    pub id: String,                          // uuidv7
    pub competency_id: String,               // FK → categories
    pub status: StoryStatus,                 // in_progress | complete
    pub current_version_id: Option<String>,  // FK → story_versions
    pub chat: Vec<ChatTurn>,                 // embedded, monotonic
    #[serde(with = "crate::models::datetime_compat::required")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoryStatus { InProgress, Complete }

pub struct ChatTurn {
    pub role: ChatRole,                      // user | assistant
    pub content: String,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub ts: DateTime<Utc>,
}
```

A Story is `complete` iff `current_version_id.is_some()`. The status enum is redundant
with that, but explicit makes the landing-page count aggregation cheap.

### `story_versions` collection

```rust
pub struct StoryVersion {
    #[serde(rename = "_id")]
    pub id: String,
    pub story_id: String,
    pub version_n: u32,                      // 1, 2, 3...
    pub body: StoryBody,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub created_at: DateTime<Utc>,
}

pub struct StoryBody {
    pub situation:  Vec<String>,
    pub task:       Vec<String>,
    pub action:     Vec<String>,
    pub result:     Vec<String>,
    pub reflection: Vec<String>,
}
```

All DateTime fields use `datetime_compat::required` like the rest of the app.

**Embedded chat vs separate collection:** chat is embedded. Worst-case chat ≈ 50 turns ×
250 chars ≈ 12 KB — well under the 16 MB doc limit. Keeps everything in one query and
eliminates a join on every render.

## Prompts

Three new prompts, seeded via `prompt_store::seed_defaults` (extend `PROMPT_NAMES`):

### `story_chat` — system prompt for every turn during the chat
- Names STAR+ as the spine, CLEAR for clarification, ADAPT for pressure follow-ups
- Hard rules:
  - Ask **one** question per turn
  - **Never** recommend, ideate, suggest answers, write content for the candidate, or rephrase what they said as a polished version
  - Track which STAR+ section is in focus; advance only when CLEAR-survivor specifics are in hand for that section, and don't leave a key decision unstressed (at least one ADAPT round on the central decision/conflict)
  - When coverage feels met across all five sections, propose verbally:
    *"I think we have a solid story — ready to lock in, or want to dig deeper anywhere?"*

### `story_summarize` — used by Generate
- Input: full chat history
- Output: JSON `StoryBody`. `force_json=true`
- 3–6 bullets per section, each ≤ 25 words, **in the candidate's voice** (no AI ideation, no embellishment, no synthesis of details the candidate didn't say)
- Sparse sections are OK — they're a signal to refine, not a defect to paper over

### `story_refine_open` — used by Continue (Option X kickoff)
- Input: current version's bullets + chat history (recent slice)
- Output: one probing question (plain text). No preamble, no summary
- Picks the thinnest section by mechanical signal: fewest bullets, vaguest language, missing follow-up depth

Files: `prompts/story_chat.md`, `prompts/story_summarize.md`, `prompts/story_refine_open.md`.
Add to `PROMPT_NAMES` and `seed_for(name)` in `services/prompt_store.rs`.

## Routes

`src/routes/stories.rs` mounted at `/stories`:

```
GET  /stories                          # landing — competency grid + counts
POST /stories                          # create — body { competency_id }, → /stories/:id
GET  /stories/:id                      # chat view + current version (if any)
POST /stories/:id/turns                # user turn → model → assistant turn (HTMX fragment)
POST /stories/:id/generate             # summarize chat → new StoryVersion, repoint current
POST /stories/:id/continue             # Option-X opening probe (refine kickoff)
POST /stories/:id/delete               # cascade delete, → /stories
GET  /stories/:id/versions/:vid        # read-only past-version view
```

Add a nav entry in `base.html` between **Practice** and **Companies**.

## Templates

```
templates/stories/index.html           # competency grid; tile = name, "X in-progress · Y complete", "+ New story"
templates/stories/show.html            # split: chat + composer | current version bullets (or "no version yet")
templates/stories/version.html         # read-only past version body, "Back to story" link
```

The chat panel uses the same HTMX patterns the rest of the app already uses: form posts
to `/turns`, server returns the new turn(s) as a fragment that appends to the chat
container.

## Phasing

### Phase 1 — Models + seed prompts
- `src/models/story.rs` with `Story`, `StoryVersion`, `StoryBody`, `ChatTurn`, status/role enums
- Pub use in `models/mod.rs`
- `prompts/story_*.md` seed files (full framework content; lift terminology from the book)
- Extend `PROMPT_NAMES` and `seed_for(name)` in `services/prompt_store.rs`
- `cargo build && cargo clippy --all-targets -- -D warnings` clean

### Phase 2 — Landing + create
- `routes::stories` skeleton with GET/POST `/stories`
- Aggregation: count stories per competency by status (one query, group by `competency_id` + `status`)
- `templates/stories/index.html` — grid of competency tiles with counts and "+ New story" buttons
- POST `/stories` creates an empty Story for the chosen competency_id, redirects to `/stories/:id`
- Nav entry in `base.html`

### Phase 3 — Chat loop
- GET `/stories/:id` — render chat history + composer (no version yet on first visit)
- POST `/stories/:id/turns` — append user turn → build `[story_chat system, ...chat history]` → `or.chat(critique_model, msgs, false)` → append assistant turn → return updated chat HTMX fragment
- `templates/stories/show.html` with chat thread + composer + (empty) version panel

### Phase 4 — Generate
- POST `/stories/:id/generate` — build summarize prompt with chat history, `or.chat(model, msgs, true)`, parse `StoryBody` (use `parse_json` from openrouter.rs), insert StoryVersion (`version_n = current+1`, or 1 first time), set `Story.current_version_id` and `status = complete`, redirect to `/stories/:id`
- Render the current version's bullets per STAR+ section in `show.html` alongside the chat

### Phase 5 — Refine kickoff (Option X)
- POST `/stories/:id/continue` — load current version bullets + last N chat turns, call `story_refine_open`, append the model's targeted probe as a new assistant turn, redirect/swap to chat
- "Continue chatting" button on the completed-story view triggers this
- Subsequent turns flow through Phase 3's loop; eventually user clicks Generate again → new StoryVersion `version_n = 2`, `current_version_id` repointed

### Phase 6 — Discard + version history
- POST `/stories/:id/delete` — cascade delete Story + StoryVersions
- GET `/stories/:id/versions/:vid` — read-only view of an older version, linked from a "v1, v2, …" picker in `show.html`

### Phase 7 — Polish (cuttable if time-pressed)
- "Coverage hint" pulse on the Generate button when the AI's last message matches the proposal phrase
- Confirm dialog on premature Generate ("AI hasn't proposed ready — generate anyway?")
- Markdown rendering for assistant messages (existing pattern from critique view, if any)
- TTS toggle for assistant turns (default off — text-first is the right modality)

## Implementation notes & gotchas

- **Smart model:** `settings.critique_model`. Don't add a new `story_model` setting until we see whether v1 needs the knob.
- **Version numbering:** `version_n` is monotonic per story; `current_version_id` is the live pointer. Old versions are immutable, never deleted on Generate (only on Discard).
- **Premature Generate:** the summarize prompt is robust to thin chat — it produces sparse bullets, which the candidate sees and can refine. Don't gate it.
- **Refine startup is one shot.** After the Option-X opening probe, regular `story_chat` orchestration resumes. The refine prompt isn't sticky.
- **No live-interview integration in v1.** Future hook: when critiquing in `/sessions`, optionally pass the candidate's matching-competency stories as additional system context. Out of scope here.
- **Deleted competency:** if a story's `competency_id` no longer resolves, render under "Unknown competency" with Delete only — same pattern as the deleted-company case in `/practice`.
- **Discipline drift watch:** if in early use the model slips into ideation ("here's how I'd phrase this for an interview…"), the fallback is to add explicit move-type tagging (Option B from the clarity round) — not a v1 concern.

## Acceptance — what "done" looks like

- I land on `/stories` and see the 12 canonical competencies as tiles with `0 in-progress · 0 complete`.
- I click **Ownership** → land on `/stories/<id>` with an empty chat. The AI opens with a probe like *"Tell me about a time you owned an outcome end-to-end. Start with what was at stake if it failed."*
- I answer. AI follows with a CLEAR clarification (*"when you say 'we', who else was involved and what was each doing?"*); cycle continues. Eventually AI says: *"I think we have a solid story — ready to lock in, or want to dig deeper?"*
- I click **Generate**. STAR+ bullets render to the right of the chat. Story shows as `complete` on the landing page.
- I click **Continue chatting**. AI opens with: *"Your v1's Reflection was abstract — 'I learned to communicate.' Let's pin down a specific signal you'd watch for now you weren't watching for then."* Chat resumes. Eventually I Generate again → v2 with richer Reflection bullets; v1 stays accessible via a `v1, v2` picker.
- I click **Discard** → confirm → Story + StoryVersions gone; landing-page counts decrement.
