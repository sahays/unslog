# Cross-company practice — plan

Locked decisions from the /clarity round:

## Shape

- **Roles** become canonical: enum `Role { SolutionsArchitect, SoftwareEngineer, EngineeringManager, ProgramManager, ProductManager }`. Each `Company` gains a `canonical_role` field; the existing freeform `role: String` stays as the company-specific name (e.g. "Applied AI Solutions Architect", "Customer Engineer at Google") and is still used in research/critique prompts.
- **Categories** are global and canonical: a single DB-seeded list of 12 entries (Ownership, Prioritization, Bias for Action, Dealing with Ambiguity, Customer/User Obsession, Collaboration & Influence, Conflict & Disagreement, Communication, Highest Standards, Diving Deep, Strategic Thinking, Failure & Learning). Editable later via a small `/categories` admin page. `Question` gains `categories: Vec<CategoryId>`.
- **Question schema** gains `role: Role` and `categories: Vec<CategoryId>`. `company_id` becomes `Option<String>` so role-only questions exist (e.g. "Tell me about a time you failed" — generic, applies to any company in the role).
- **Settings** adds `lite_model: String` for the cheap classifier used by the categorizer and curator. Default `google/gemini-2.5-flash`. Snapshotted onto sessions at start.
- **Sessions** become curated: at start, an LLM call (using `lite_model`) picks 4–6 questions in order from the role-and-company-filtered pool, biased by the user's recent recurring weaknesses and excluding recently-asked questions. The full list is stored on the session as `curated_question_ids: Vec<String>` plus a one-line `focus_line: String` ("Today's focus is ambiguity, where you stumbled last session, plus a fresh ownership story for breadth"). `next-question` now just iterates through the curated list. After the last curated question is answered, the session auto-ends → debrief.
- **Session entry points** stay both (single-company and cross-company), running through the same curator. From `/companies/:id`, scope = that one company. New top-level `/practice` route picks role + multi-company for cross-company shuffle.
- **Critique context**: source-company packet when the question has one. For role-only questions, drop the "company fit" axis (`Critique.scores.company_fit` becomes `Option<u8>`); critique prompt is conditional on company presence.
- **No migration**. Fresh start. Existing companies will be deleted and recreated.
- **All long-running operations** keep using the Phase M modal overlay with operation-specific copy.

## What this is NOT

- Not adding a categories picker on session start. The user picks role + companies; the curator handles category balance silently.
- Not building per-company LP/value lists. The 12 canonical categories cover all companies; cultural nuance lives in the company research packet.
- Not building a deterministic-shuffle curator. LLM curator only — adds 1–3s of latency at session start, but produces narrative coherence and weakness-aware picks that rules can't.
- Not migrating existing data. User will delete + recreate companies.
- Not changing the heavy models (critique, research, STT, TTS) — those keep their current settings.

## Phases

Each phase ends with a clean `cargo build` + clippy + smoke loaded pages, and a single commit pushed to `origin/main`. No Co-Authored-By trailers.

### Phase 1 — Roles + Categories data foundation

Goal: introduce the canonical Role enum and the canonical Category collection without touching any flows yet.

- New enum `Role` in `src/models/role.rs`: 5 variants with `as_str()` / `from_str()` / `display_name()` / `all() -> &[Role]` helpers and `Serialize` / `Deserialize` for BSON.
- New model `Category` in `src/models/category.rs`: `{ _id, name, description, sort_order, created_at }`. Collection: `categories`.
- New service `src/services/category_store.rs`:
  - `seed_defaults(db)` — on first run inserts the 12 canonical entries (only if collection is empty)
  - `list_all(db)` — sort by `sort_order ASC, name ASC`
  - `get(db, id)`, `save(db, cat)`, `delete(db, id)`
- DB indexes (`src/db/mod.rs`): unique on `categories.name`.
- `Company` gains `canonical_role: Role` (BSON serialized as snake_case string). `Company::new(...)` updated to take it.
- `Question` (in `src/models/question_bank.rs`): gains `role: Role` and `categories: Vec<String>` (CategoryId list). `company_id` on the Question stays — questions are still inside a company's bank for now; the role-only-question case falls out from a question whose `company_id` matches a virtual "global" bank in Phase 5. Defer that until Phase 5.
- `startup::run` calls `category_store::seed_defaults` after `prompt_store::seed_defaults`.

**Done when**: `categories` collection has 12 rows on a fresh DB; companies and questions have new fields visible in mongo; existing UI still renders (no template touches yet).

### Phase 2 — Lite model in settings

Goal: add the cheap classifier slot to settings, plumb through to ModelSnapshot.

- `Settings` gains `lite_model: String`. `settings_store::load` defaults to `google/gemini-2.5-flash` on first read.
- `ModelSnapshot` gains `lite: String`. `routes/sessions::start` snapshots `settings.lite_model` into it.
- `routes/settings::show` adds `lite_model` to the SettingsTemplate; `ensure_present` covers it. Filter: same `is_preferred` chat-models list (gemini/openai/anthropic/deepseek prefixes).
- `templates/settings/index.html` adds a 5th model picker: "Lite model — used for fast tagging + question curation. Pick a cheap fast model." Same NSelect-allow-custom pattern.
- `routes/settings::save` validates `lite_model` non-empty.

**Done when**: settings page shows 5 model pickers, save round-trips, mongo `settings.default` doc has the new field.

### Phase 3 — Auto-categorize on company create/refresh

Goal: when the research agent generates sample questions, tag each with canonical categories via a `lite_model` call. Wire into create + refresh.

- New service `src/services/categorize.rs`:
  - `pub async fn categorize_question(or, lite_model, question_text, categories) -> Vec<CategoryId>` — single-question call, returns 1–3 tags
  - `pub async fn categorize_questions_batch(or, lite_model, question_texts, categories) -> Vec<Vec<CategoryId>>` — batch variant: builds one prompt with all the questions, parses one structured JSON response back. Cheaper than N individual calls.
  - Prompt shape: system asks the model to act as a behavioral-interview competency classifier; user message lists the canonical categories with descriptions and the questions. JSON output: `[{ "question_index": 0, "categories": ["ownership","prioritization"] }, ...]`.
- `services/research.rs::run`: after producing the `ResearchPacket`, return the (text, categories) pairs by passing the sample_questions through `categorize_questions_batch` (graceful-fail: if the lite call errors, fall back to empty category lists; questions are still saved, just untagged).
- `routes/companies::create`:
  - Pass canonical_role from form (new field — see Phase 4 for UI; for Phase 3 just accept it from the form data).
  - After research, persist Question rows with `role = company.canonical_role`, `categories = <agent tagged>`, `company_id = company.id`.
- `routes/companies::refresh_packet`: re-tag the new questions only; existing questions keep their tags (don't blow away user edits).
- `services/research.rs` span gets a child span `"categorize"` with `duration_ms`, `count`, `categories_assigned_total`.
- Overlay copy on the company form: "Researching the company and tagging questions…" (Phase M overlay machinery).

**Done when**: creating a new company produces sample questions in mongo with non-empty `categories` arrays; logs show `categorize` event with timing.

### Phase 4 — Roles dropdown + manual question editing UI

Goal: surface canonical roles on company creation and let the user manage questions (role + categories) on the company show page.

- `templates/companies/list.html`: company create form gains a `<select data-n-select>` with the 5 canonical roles + freeform `name` input renamed to "Specific role name (optional)".
- `routes/companies::create` form binding accepts `canonical_role` (parses from `Role::from_str`).
- `templates/companies/show.html`:
  - Each question row shows category badges (from the 12 canonical, color-coded subtle teal)
  - "Edit" affordance per question → inline form (or popover) that edits text + role (default = company's canonical_role) + categories (multi-select via NSelect or checkbox grid; 12 fits cleanly).
  - The paste-questions textarea now auto-tags pasted lines via `categorize::categorize_questions_batch` on insert (overlay shows "Tagging questions…").
- `routes/companies::add_questions` extended: take the pasted lines, call categorize, save Question rows with role + categories.
- New route: `POST /companies/:id/questions/:qid/edit` for inline edits.

**Done when**: pasted questions land tagged; clicking "Edit" on a question shows the categories multi-select with current values; saving updates mongo.

### Phase 5 — Session curator + schema rework

Goal: replace the random `pick_next` with an LLM curator that picks 4–6 questions in order at session start. Sessions auto-end when the curated list is exhausted.

- `Session` schema:
  - Add `role: Role`
  - Add `selected_company_ids: Vec<String>` (single-company sessions store one ID)
  - Add `curated_question_ids: Vec<String>` (ordered list)
  - Add `focus_line: String`
  - Drop `current_question_id` / `current_question_text` / `current_question_audio_path` — derive from curated list + answered count instead. Or keep for backwards compatibility but recompute on each `next_question`. **Decision**: keep them, just set them from curated list cursor. Simpler templates.
- New service `src/services/curator.rs`:
  - `pub async fn curate(state, session_inputs) -> CuratorOutput`
  - Inputs: role, selected_company_ids, db
  - Loads candidate pool: `questions WHERE role = X AND company_id IN selected_company_ids` (Phase 5 only handles company-scoped questions; role-only questions land in Phase 7 if needed)
  - Loads recent: last 3 ended sessions for this role, extracts `summary.recurring_weaknesses` and recently-asked `question_ids`
  - Calls `lite_model` with structured JSON output: `{ "question_ids": [...], "focus_line": "..." }` — picks 4–6 in order, biased toward weakness-categories, excluding recently-asked. Pool > 6: prefer fresh + diverse. Pool 4–6: take all, in coherent order. Pool < 4: take all (warn).
  - Output: `CuratorOutput { question_ids, focus_line }`
- `routes/sessions::start`:
  - Form now takes `role` (canonical, derived from company.canonical_role for single-company entry) + `selected_company_ids` (just `[company.id]` for single-company)
  - Calls `curator::curate`, stores `curated_question_ids` and `focus_line` on the session
  - Sets `current_question_id` from `curated_question_ids[0]` (and TTS the first question)
- `routes/sessions::next_question`:
  - Walks the curated list: find current_question_id's index, advance to next.
  - If we've answered the last one, auto-redirect to `/sessions/:id/end` (which generates summary).
  - TTS the new current question.
- `routes/sessions::submit_answer`:
  - After saving the Nth answer (where N == curated count), trigger end flow automatically (POST /end inline) instead of redirecting back to the active page.
- Active session page header shows `session.focus_line` ("Today's focus: …") below the company/role line.
- Overlay copy on company "Start session" form: "Curating today's session… picking 4–6 questions tailored to past weaknesses."

**Done when**: clicking Start on a company creates a session with a non-empty `curated_question_ids` array and a `focus_line`; the active page header shows the focus line; answering all curated questions auto-ends the session.

### Phase 6 — Top-level /practice (cross-company)

Goal: new entry point for cross-company sessions.

- `templates/practice/index.html`: form with role dropdown + multi-company checklist (filtered to companies of the picked role; default = all matching).
- `routes/practice.rs`:
  - `GET /practice` — render the form
  - `POST /practice` — validate, build session with `role` + `selected_company_ids` (multiple), call curator, redirect to `/sessions/:id`.
- `templates/base.html` nav: new "Practice" entry between Companies and Assets, lucide icon `target`.
- Overlay copy: same curator wait, with detail mentioning the selected companies count.

**Done when**: `/practice` page renders, picking role + companies + Start lands you in a curated session pulling across all selected companies.

### Phase 7 — Role-only questions + critique adapts

Goal: support questions with no source company (`company_id = None`) and update the critique prompt + JSON schema to handle the missing "company fit" axis.

- `Question.company_id`: change from `String` to `Option<String>`. Migration not needed (fresh start).
- `Critique.scores.company_fit`: change from `u8` to `Option<u8>`. JSON schema in critique.md updated to allow null/omit when no company packet provided.
- `services/critique.rs`:
  - When question has no source company, skip the company packet block in the user message and instruct the model to omit `company_fit` (or set to null).
  - When question has a source company, behaviour unchanged.
- Templates: anywhere `c.scores.company_fit` is rendered, gate with `if let Some(...)` and skip the badge when absent.
- A small "Add role-only question" affordance on the new `/practice` page (or a separate `/role-questions/<role>` page) — defer the UI to a future phase if scope creeps; backend support lands here.
- `services/curator.rs`: extend pool query to include `company_id IS NULL` for role-only questions; surface them in the same curated mix.

**Done when**: a question with no `company_id` can be picked by the curator, gets through the answer flow, and produces a critique JSON that the parser handles without `company_fit`.

### Phase 8 — Polish + categories admin

Goal: complete the categories admin UI and fix the rough edges.

- `templates/categories/list.html`: simple admin page listing the 12 canonical entries with edit/add/delete.
- `routes/categories.rs`: GET list, POST create, POST :id/edit, POST :id/delete (delete cascades nothing; questions tagged with the deleted category just lose that tag).
- Settings page validation: ensure `lite_model` matches one of the chat-capable filtered models or falls through to the allow-custom freetext (already covered by NSelect).
- Empty-state handling on `/practice`: if no companies match the picked role, show a clear message + CTA to add one.
- Curator graceful-fail: if `lite_model` errors and the curator can't return picks, fall back to a simple random pick of 5 from the pool with a warn log + a placeholder focus_line ("Random selection — curator unavailable.").
- Top-of-page `focus_line` styling — quoted, italic, in the muted-soft tone.
- `cargo build --release` clean; `cargo clippy --all-targets -- -D warnings` clean.

**Done when**: every loose end is closed, click-through tour from create-company → start-cross-company-practice → answer 5 → debrief works without warnings.

### Phase 9 — Questions as a separate collection (post-plan addition)

Goal: drop the embedded `QuestionBank.questions[]` array. Make Question a top-level MongoDB document with `company_id: Option<String>` so role-only questions are first-class.

- Drop `models/question_bank.rs` and `services/question_bank.rs` entirely.
- New `models/question.rs`: `Question { id, text, source, role, categories, company_id: Option<String>, added_at }`. `COLLECTION = "questions"`.
- New `services/questions.rs`:
  - `list_for_company(db, company_id)` — sorted by added_at
  - `list_for_pool(db, role, &[company_ids])` — `role = X AND (company_id IN ids OR company_id IS NULL)`. Used by curator.
  - `get(db, id)` — direct lookup, no per-company walk
  - `append(db, texts, source, role, company_id, categories_for_fn)` — bulk insert
  - `update(db, id, text, role, &[categories])`
  - `delete(db, id)`
  - `delete_for_company(db, company_id)` — cascade on company delete
  - `pick_random(&pool, &seen)` — fallback for legacy sessions
- DB indexes: `(company_id, added_at)`, `(role,)` on the new `questions` collection. Drop the old `question_banks.company_id_unique` index (collection unused; mongo will leave it alone, harmless).
- `routes/companies::create` → questions::append with company_id.
- `routes/companies::add_questions` (paste) → questions::append.
- `routes/companies::delete_question` → questions::delete by id (no need for company-scoped delete now).
- `routes/companies::edit_question` → questions::update.
- `routes/companies::refresh_packet` → list_for_company → diff → append.
- `routes/companies::delete` (company) → questions::delete_for_company cascade.
- `routes/companies::show` → ShowTemplate.questions: Vec<Question> instead of bank: QuestionBank.
- `templates/companies/show.html` → iterate `questions` directly (was `bank.questions`).
- `services/curator::load_pool` → questions::list_for_pool — naturally includes role-only questions whose company_id is null.
- `routes/sessions::load_question_by_id` → direct collection lookup, no per-company walk.
- `routes/sessions::next_question` legacy fallback → questions::list_for_company + questions::pick_random.

**Done when**: company show page lists questions; deleting a company cascades; curator's pool query naturally includes role-only questions when any exist; build clean; clippy clean; release build clean.

UI for creating role-only questions (no company) is still deferred to a follow-up. The data model supports them; user can add them by inserting directly into mongo or wait for an admin page.

## Out-of-scope follow-ups (not in this plan)

These are tempting during the rebuild but explicitly **not** part of it:

- Per-question difficulty scoring (the curator currently doesn't know which questions are harder)
- Streak/progression tracking across sessions (e.g. "you've improved on prioritization over your last 3 sessions")
- Auto-suggest companies to add based on the role (research agent could fetch competitors, etc.)
- Sharing curated session links

Each is a separate small feature once this plan is done.
