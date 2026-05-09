# Three-Column Layout — Implementation Plan

## Goal

Replace the current top-nav layout with an app-wide three-column shell:
left sidebar (nav), thin top bar (breadcrumbs + theme toggle), main content
column (max-w-4xl), right rail (page-specific context). Mobile-first
progressive enhancement: single-column → two-column at lg → three-column at xl.

## Decisions (settled in /clarity)

| Decision | Choice |
|---|---|
| Top nav strategy | Replace the existing top nav entirely with a left sidebar; keep a thin top bar for breadcrumbs + theme toggle |
| Right rail scope | Always present app-wide; route-specific content where natural, empty placeholder elsewhere |
| Right rail rollout | Layout-first; wire explicit content where it's clearly useful (stories, sessions, companies, practice). Empty placeholder elsewhere — design later |
| Responsive cascade | Mobile (default): single column + hamburger drawer. lg+: sidebar visible, two columns, no rail. xl+: three columns including rail |
| Rail fallback below xl | None — rail content disappears entirely (does *not* fold back inline) |
| Breadcrumbs | Full hierarchy with entity names: "Stories / Ownership", "Companies / Anthropic", "Practice / Anthropic / May 8 session". Single crumb for flat routes |
| Brand mark | Top of left sidebar (not top bar) |
| Theme toggle | Top bar, right-aligned |
| Sidebar grouping | Flat list of 6 items — grouping not earned at this count |
| Mobile drawer | Modal overlay with backdrop, not push-content |
| Out of scope (v1) | Recent-activity feed in the rail's empty state, sidebar collapse-to-icons toggle, sub-route navigation inside sidebar |

## Layout shell architecture

`templates/base.html` is the single source of truth for the shell. New
block API every page can fill:

```jinja
{% block title %}{% endblock %}      {# already exists #}
{% block breadcrumbs %}{% endblock %} {# new: per-page breadcrumbs ul/list #}
{% block content %}{% endblock %}     {# already exists — main column #}
{% block rail %}{% endblock %}        {# new: right-rail content; empty → placeholder #}
```

Pages provide `{% block breadcrumbs %}` as a list of crumb pairs, e.g.:

```html
{% block breadcrumbs %}
  <a href="/stories">Stories</a>
  <span>{{ competency.name }}</span>
{% endblock %}
```

base.html renders the children separated by chevron icons, last crumb un-linked.

## Responsive grid (mobile-first)

```css
.app-shell {
  display: grid;
  grid-template-columns: 1fr;
  grid-template-areas: "main";
}

@media (min-width: 1024px) {  /* lg */
  .app-shell {
    grid-template-columns: 14rem 1fr;
    grid-template-areas: "sidebar main";
  }
}

@media (min-width: 1280px) {  /* xl */
  .app-shell {
    grid-template-columns: 14rem minmax(0, 1fr) 16rem;
    grid-template-areas: "sidebar main rail";
  }
}
```

- **Sidebar**: `position: sticky; top: 0; height: 100vh` so it stays in
  view while main scrolls. On mobile, sidebar is `position: fixed; left: -100%; transition` and slides in via Alpine state when hamburger toggled.
- **Top bar**: `position: sticky; top: 0; z-index: var(--z-sticky)` —
  thin (~3rem), inside the main column on lg+, full-width on mobile.
- **Main column**: max-w-4xl (`56rem`) centered within its grid area
  with `mx-auto` and horizontal padding. The grid area itself is wider
  on big screens; the content stays comfortably readable.
- **Rail**: `position: sticky; top: 3rem; max-height: calc(100vh - 4rem); overflow-y: auto`. Quietly disappears below xl.

## Sidebar contents

```
┌───────────────────┐
│ ▣ unslog          │  ← brand mark (top)
├───────────────────┤
│ ◯ Companies       │
│ ◯ Practice        │  ← active route shows accent color + left border
│ ● Stories         │
│ ◯ Assets          │
│ ◯ Prompts         │
│ ◯ Settings        │
│                   │
│       (filler)    │
└───────────────────┘
```

Active-route detection: each route handler exposes a `nav_active` string
("stories", "practice", etc.); base.html compares against the link's key
and applies an `.active` modifier class.

Cleanest implementation: add `nav_active: &'static str` to a tiny
`LayoutContext` struct that every template embeds, OR thread it through
manually per-template. Manual is simpler for v1 — every page handler
already builds its own template struct, just add one field.

## Top bar contents

| Position | Mobile | lg+ |
|---|---|---|
| Left | Hamburger button | Breadcrumbs (left-aligned, truncate as needed) |
| Center | Breadcrumbs (truncated to fit) | — |
| Right | Theme toggle | Theme toggle |

Breadcrumb separator: lucide `chevron-right` 14px, muted color.

## Right rail per route (initial wiring)

| Route | Rail content | Source |
|---|---|---|
| `/` | empty placeholder | — |
| `/companies` | empty placeholder | — |
| `/companies/:id` | Past sessions for this company | already inline; move to rail |
| `/practice` | In-progress sessions | already inline; move to rail |
| `/sessions/:id` | Question/answer history | currently a tab; move to rail, drop the History tab |
| `/sessions/:id/review` | Question/answer history (same) | move from inline cards |
| `/stories` | empty placeholder | — |
| `/stories/:id` | Other stories for this competency + New story button | already built as inline aside (Phase X above); move to rail |
| `/stories/:id/versions/:vid` | Story version picker | already inline; move to rail |
| `/assets`, `/prompts`, `/settings` | empty placeholder | — |

Empty-state content: a muted `<p>` reading something like "Context for
this view will appear here as you use the app." Nothing more — we agreed
in `/clarity` not to invent a recent-activity feed for v1.

## Phasing

### Phase 1 — Shell skeleton
- Refactor `base.html`: grid container with three areas, sticky sidebar, sticky top bar
- Move brand mark from top nav to sidebar top
- Render the 6 nav links in sidebar with active-state styling
- Empty `{% block breadcrumbs %}` and `{% block rail %}` blocks defined
- New CSS: `.app-shell`, `.app-sidebar`, `.app-topbar`, `.app-rail`, `.nav-link-side` (sidebar variant of existing `.nav-link`)
- Mobile hamburger button + Alpine state to toggle drawer + backdrop
- Verify: every existing route renders correctly with empty rail and a single-crumb breadcrumb derived from the page title; no regressions

### Phase 2 — Breadcrumbs across all routes
- Add per-handler breadcrumb data to each route that drills into an entity
- `/stories/:id`, `/companies/:id`, `/sessions/:id` etc. all populate `{% block breadcrumbs %}` with their hierarchy
- Flat routes (`/assets`, `/prompts`, `/settings`, `/`) just render their page title as a single crumb (the existing page H1 stays — breadcrumbs are wayfinding, not the heading)

### Phase 3 — Active sidebar route
- Each route handler passes `nav_active: &'static str` to its template
- Template forwards it to base via a sidebar-render macro (or each page passes it through the template struct)
- Active link styled with accent color + left-border treatment

### Phase 4 — Migrate `/stories/:id` rail
- Move the existing `<aside class="story-side-panel">` markup into `{% block rail %}`
- Delete the `.story-side-panel` fixed-positioning CSS rule (rail container handles positioning now)
- The inline-fallback-on-narrow disappears; below xl, the panel is simply absent (per /clarity decision)

### Phase 5 — Wire `/companies/:id`, `/practice`, `/sessions/:id`
- Move each route's contextual companion into `{% block rail %}`
- `/practice`: in-progress sessions list moves out of the page top, rail takes it
- `/companies/:id`: session history list moves to rail
- `/sessions/:id`: drop the "History" tab from the tab strip; the same content renders in the rail
- Keep the inline patterns gone — don't double-render

### Phase 6 — Cleanup
- Delete the old top nav markup from base.html (already replaced by sidebar in Phase 1, but leftover utility classes can go)
- Remove any nav-link CSS that's exclusive to the horizontal top nav
- Verify dark mode + theme toggle still work (toggle moved to top bar)
- Check all existing routes for visual regressions

### Phase 7 — Polish (cuttable)
- Sidebar nav-link tooltips on truncated labels (none right now, but if labels grow)
- Smooth slide-in for mobile drawer (transform translateX, opacity backdrop)
- ESC closes mobile drawer
- Trap focus inside drawer when open
- "Skip to main content" anchor for keyboard users

## Implementation notes & gotchas

- **Don't let the rail break the page.** Use `min-w-0` on the main column's grid cell so wide content (long inline code, prose) doesn't blow out the layout — Tailwind grid items default to `min-width: auto` which forces them to fit content.
- **Sticky positioning needs an explicit `top` value.** Without it the sticky behavior silently fails. Set `top: 0` on sidebar/top-bar, `top: 3rem` (or however tall the top bar is) on rail.
- **Sidebar height = 100vh, not 100% of parent.** Otherwise it shrinks on short pages.
- **Mobile drawer state lives at the body level.** Alpine `x-data` on `<body>` so child components (the page handler can put a hamburger anywhere) can dispatch open/close.
- **Don't move dark-mode toggle's Alpine state.** It already lives on `<html>` — leave that, just relocate the trigger button into the new top bar.
- **Z-index ordering**: top bar must sit above the rail (which is sticky too). Sidebar drawer overlay above everything when open. Existing `--z-sticky` (20), `--z-modal` (40) tokens cover this — drawer = modal, sidebar/topbar/rail = sticky.
- **Existing `chat-composer-card`**: the fixed composer's `inset-x-0` currently spans full viewport. Once the layout has a sidebar at lg+, the composer should respect the main column's bounds — change `inset-x-0` to be relative to the main column, or keep the wrapper `pointer-events-none` and let the form's `max-w-3xl mx-auto` self-center within the available main-column width.
- **`/stories` landing**: the per-tile click flow we just built (tile = link to latest story OR form to create one) survives unchanged. The rail is empty on the landing page; that's fine.

## Acceptance — what "done" looks like

- I land on any route and see a left sidebar with the unslog wordmark on top and 6 nav links below; current route is highlighted.
- A thin top bar across the top of the main column shows breadcrumbs on the left ("Stories / Ownership") and the dark-mode toggle on the right.
- On `/stories/:id` at xl width, the right rail shows other stories for this competency with a "+ New story" button at the top.
- I narrow the browser to ~1100px (between lg and xl): the right rail disappears, sidebar stays.
- I narrow further to ~700px (below lg): sidebar collapses, top bar grows a hamburger button on the left, breadcrumbs squeeze in. Tapping the hamburger slides the sidebar in over a backdrop; tapping the backdrop or a nav link closes it.
- On `/practice`, the in-progress sessions list is in the rail (not at the top of the main column anymore). On `/sessions/:id`, the History tab is gone and Q&A history shows in the rail. On `/companies/:id`, past sessions show in the rail.
- Dark mode toggle still works from its new home in the top bar.
- All existing forms and flows behave exactly as before.
