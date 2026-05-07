# UI redesign plan

Locked decisions from the /clarity round:

- **Scope**: Full restyle (typography, palette, macros, strip every inline `style="..."`).
- **Reference**: Glimmer's bones — frosted sticky top nav, gradient progress bar, macros pattern, dark-mode token system. Layout shape kept as-is (no sidebar).
- **Identity**: Deliberate "deep practice" palette, *not* glimmer's purple-pink.
  - Light: accent `#0d6e6e` (deep teal) on warm-paper bg `#faf8f3`, ink text.
  - Dark: candlelit warm dark — bg `#1a1610`, accent lifted to brass-teal `#4a9c8c`.
- **Typography**: Bricolage Grotesque (headings), Lexend (body), JetBrains Mono (code, transcripts).
- **Icons**: Lucide via `<script src="…lucide@latest">` CDN, replacing text-only buttons.
- **Active session page** — tabbed: `Current question` / `Session history (N)` / `Debrief` (latter visible only when ended). Recording survives a tab switch; the active tab gets a red dot when recording.
- **All other pages** — restyle only, no IA changes.

## What this is NOT

- No new features. No new routes. No new database fields.
- No marketing/landing pages. unslog stays a tool.
- No mobile menu — single-user desk tool. Pages must be readable on a narrow viewport but a hamburger nav is overkill.
- No animation library beyond what glimmer's CSS already provides (CSS transitions; no GSAP / ScrollTrigger).

## Phases

Each phase ends with a clean `cargo build` + clippy + smoke-loaded pages, and a single commit pushed to `origin/main`. Per past habit, no Co-Authored-By trailers.

### Phase A — CSS foundation

Replace the entire `static/css/input.css` palette + tokens, keeping the Tailwind v4 + `@custom-variant dark` setup unslog already uses.

- Lift glimmer's `@theme` neutral scale (ink-50…950) verbatim — these are the surface/text greys and they're already what unslog half-uses.
- Replace glimmer's purple/pink scales with a `teal-*` (50–950) scale anchored on `#0d6e6e`/`#4a9c8c`. Drop the pink scale entirely; nothing in unslog is "secondary CTA".
- Light tokens: `--bg #faf8f3`, `--bg-soft #ffffff`, `--bg-sunk #f1ede4`, `--fg #1a1f2c`, `--fg-muted #647088`, `--border #e6e0d2`, `--accent #0d6e6e`, `--accent-h #0a5a5a`, `--good #1f9c6c`, `--danger #b94a44`.
- Dark tokens: `--bg #1a1610`, `--bg-soft #221c14`, `--bg-sunk #14110b`, `--fg #f6f0e0`, `--fg-muted #a89d83`, `--border #322a1e`, `--accent #4a9c8c`, `--accent-h #62b3a3`.
- Lift glimmer's component classes (`.nav-frosted`, `.btn-primary` gradient, `.card-interactive`, `.chip-ai`, link hover animations) and re-color them to teal.
- Add Google Fonts `@import` for Bricolage Grotesque, Lexend, JetBrains Mono (in `base.html` `<head>`, not in CSS, so the CSS file stays cacheable).
- Add `kbd` styling, retained from current Phase 13 work.

**Done when**: `cargo build` regenerates `static/css/app.css`, all existing pages still render (now with teal accents on top of the old layout), and a manual eyeball confirms the body font is Lexend and headings are Bricolage Grotesque.

### Phase B — Macros & base shell

Bring glimmer's macro pattern into unslog, adapted to Askama.

- New files:
  - `templates/macros/ui.html` — `card_header(icon, title)`, `badge_default/success/warning/error/accent`, `alert_info/success/warning/error`, `stat_card(label, value)`, `link(href, text)`, `link_btn(href, text, icon)`.
  - `templates/macros/forms.html` — `input(name, label, type, value, placeholder)`, `textarea(name, label, value, placeholder, rows)`, `select(name, label, options, selected)`, `submit(text, icon, variant)`, `file_input(name, label, accept)`. Each renders the label + control + error slot consistently.
  - `templates/macros/grid.html` — `page_header(title, subtitle, actions_block)`, `section(title, body_block)`, `two_col(left, right)`. These give every page the same vertical rhythm.
- Rewrite `templates/base.html`:
  - Frosted sticky nav (`.nav-frosted`), Lucide-icon links to Companies / Assets / Prompts / Settings, dark-mode toggle reusing existing Alpine `darkMode` x-data.
  - Gradient progress bar at top — teal → brass-teal → teal (replaces the current 2px solid bar).
  - Add `<script src="https://unpkg.com/lucide@latest">` and a small `<script>` that calls `lucide.createIcons()` on `DOMContentLoaded` *and* on `htmx:afterSwap` (so swapped HTML re-renders icons).
  - Add Google Fonts `<link>` preconnect + stylesheet.
  - Wrap `{% block content %}` in a `max-w-5xl mx-auto px-6 sm:px-8 py-8` container — the standardized page frame.
- Move the existing `data-recorder-root` / Alpine init expectations forward unchanged (recorder.js stays).

**Done when**: Every existing page boots inside the new shell. They'll look unstyled internally (still inline styles), but the chrome is glimmer-grade.

### Phase C — Restyle simple pages (no IA changes)

Strip every inline `style="..."` and rebuild using Tailwind utility classes + the new macros. Order is "easiest first to build muscle":

1. `templates/home.html` — keep the dashboard layout from Phase 13. Use `stat_card` for the four stat tiles, `link_btn` for the CTAs, `card_header` for the recent-sessions section.
2. `templates/settings/index.html` — use `forms::input` and the `select` component for the model pickers; group the four model fields under `card_header(icon="sliders", title="Model selection")`.
3. `templates/prompts/list.html` and `templates/prompts/edit.html` and `templates/prompts/history.html` — `card_header(icon="file-text")`, `forms::textarea` for the body editor, `badge_*` for "current version" / "restored from".
4. `templates/assets/list.html` and `templates/assets/preview.html` — `card_header(icon="book-open")`, `forms::file_input` for upload, `badge_success/warning/error` for extraction status.
5. `templates/errors/*.html` — `alert_error` macro.

For each page: zero inline styles, zero raw `style="..."` attrs, all spacing via Tailwind utilities, all interactive elements with proper `hover:` and `dark:` variants, all icons via Lucide.

**Done when**: All five page-groups use only utility classes + macro calls; `grep -rn 'style="' templates/` for these files returns empty.

### Phase D — Restyle data pages

Same treatment, slightly more work because they have lists and tables.

1. `templates/companies/list.html` — research-state `badge_*`, primary CTA `link_btn(icon="plus")`, list rows as `card-interactive` rows.
2. `templates/companies/show.html` — `card_header(icon="briefcase")` for the company panel, the past-sessions list using `components/table.html` (lifted from glimmer), the research packet section with `card_header(icon="search")`.
3. `templates/sessions/review.html` — `card_header(icon="clipboard-list")`, score display rebuilt from inline tags into proper `badge_*` rows, audio players inside a `bg-bg-sunk rounded p-3` container.

**Done when**: same `grep -rn 'style="'` cleanliness check; companies show page renders cleanly with a real company that has past sessions.

### Phase E — Active session: tabbed

The big one. `templates/sessions/active.html` becomes a three-tab page.

- Tab strip implementation: Alpine.js `x-data="{ tab: 'current' }"` on the page wrapper, `:class` toggles for active-tab styling, `x-show` for panel visibility. No HTMX swap — the data is already on the page.
- Default tab: `current` if active session and there's a current question, otherwise `history` if history exists, otherwise `current` (empty state). When ended, default to `debrief`.
- Tab labels with Lucide icons:
  - `Current question` (icon: `mic`) — visible only when session is active. Shows the current-question card, attempts, answer form, recorder, voice toggle.
  - `Session history (N)` (icon: `history`) — always shown. N = total questions answered, including the current one. Empty state: "No questions yet — answer one to see it here."
  - `Debrief` (icon: `clipboard-check`) — visible only when session is `Ended` and a `Summary` row exists. Shows narrative + strengths/weaknesses/blind-spots/fit-signal cards.
- **Recording survives tab switch**: the recorder DOM (`data-recorder-root`) lives inside the `current` tab panel, but the panel is rendered with `x-show` (CSS `display:none`), not `x-if` (DOM removal). MediaRecorder keeps running; the active tab's label gets a red `●` next to it whenever `recording === true` (Alpine reads from a global `window.unslogRecording` flag that recorder.js toggles). This requires a tiny patch to `static/js/recorder.js`: after `setState`, set `window.unslogRecording = recording` and dispatch a `unslog:recording` CustomEvent so Alpine can listen.
- Page header: company name + role on the left, action buttons on the right (`Back`, `Full review`, `End session`) — using `link_btn_secondary` and a `btn-danger` variant.
- Keyboard shortcuts (already implemented) keep working — they don't care which tab is visible.

**Done when**: an end-to-end session works: start → answer → critique appears → switch to History tab and see the answered question → switch back to Current → record next → end session → Debrief tab appears and is the new default. Recording continues if you click another tab mid-record (verified by stopping later and seeing the transcript land).

### Phase F — Polish & verify

- Find every remaining inline `style="..."` (`grep -rn 'style="' templates/`) and migrate. Should be zero after E.
- Lucide icon audit: every action button has an icon; every empty state has a muted icon hero. No bare-text buttons except inside dense lists where the column is too narrow.
- Run the server, click through every page in light mode and dark mode. Take notes on any color that reads wrong; tweak token values in `input.css`.
- Run `cargo clippy --all-targets -- -D warnings`.
- Run `cargo build --release` to confirm the Tailwind production build works (some classes only get included if they appear in a template — re-build is the only reliable check).
- Single squash-style commit per phase, pushed.

## Out-of-scope follow-ups (not in this plan)

These are tempting during the redesign but explicitly *not* part of it:

- Score-progression chart on the home dashboard. (Phase 11+ data is there, but charting needs a charting decision and a small JS lib.)
- Per-question filtering on the company show page.
- Markdown rendering of the critique narrative (today it's plain text).
- Toast notifications for save/error events.

Each is a separate small feature once the redesign is done.
