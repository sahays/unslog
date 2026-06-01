//! Route → cost-unit table.
//!
//! Pure. No I/O, no state. The metering middleware looks the weight up
//! before the handler runs; the table-driven shape keeps every billed
//! endpoint visible in one place and makes the coverage test (which walks
//! the assembled router) easy to keep honest.
//!
//! A weight of `0` means "free / read-only" — the cap-check short-circuits
//! and only the log row is appended. Anything `> 0` is enforced.

use axum::http::Method;

use crate::models::RequestKind;

/// One row of the table. `template` may carry `:name` segments which match
/// any non-empty actual segment. Kept as a `(Method, &str, RequestKind, u32)`
/// tuple so the table itself is a `const` slice — no allocation at startup.
type Entry = (HttpMethod, &'static str, RequestKind, u32);

/// Mirror of `axum::http::Method` for the variants we actually route. The
/// table is `const`-able only if every entry is itself a `const`-buildable
/// value; the upstream `Method` constants aren't `const` for `PartialEq`,
/// so this small enum bridges the gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl HttpMethod {
    fn matches(self, m: &Method) -> bool {
        matches!(
            (self, m.as_str()),
            (HttpMethod::Get, "GET")
                | (HttpMethod::Post, "POST")
                | (HttpMethod::Put, "PUT")
                | (HttpMethod::Patch, "PATCH")
                | (HttpMethod::Delete, "DELETE")
        )
    }

    pub fn as_axum(self) -> Method {
        match self {
            HttpMethod::Get => Method::GET,
            HttpMethod::Post => Method::POST,
            HttpMethod::Put => Method::PUT,
            HttpMethod::Patch => Method::PATCH,
            HttpMethod::Delete => Method::DELETE,
        }
    }
}

use HttpMethod::*;
use RequestKind::*;

/// Billed endpoints. Verified against the assembled router by
/// `tests/metering_route_coverage.rs`; the coverage test fails if a route
/// is added without a billing decision.
const WEIGHTS: &[Entry] = &[
    // ── Sessions ─────────────────────────────────────────────────
    // Submitting an answer chains STT → critique LLM → TTS.
    (Post, "/sessions/:id/answers", Llm, 5),
    // STT-only sub-step.
    (Post, "/sessions/:id/transcribe", Stt, 2),
    // End of session triggers a summary generation.
    (Post, "/sessions/:id/end", Llm, 2),
    // Re-synthesise critique audio for a stored attempt.
    (
        Post,
        "/sessions/:id/attempts/:eval_id/:n/regenerate-audio",
        Tts,
        1,
    ),
    // Session lifecycle that runs the curator + first-question TTS.
    (Post, "/companies/:id/sessions", Llm, 2),
    (Post, "/sessions/:id/next-question", Llm, 1),
    // ── Stories ──────────────────────────────────────────────────
    (Post, "/stories/:id/turns", Llm, 1),
    (Post, "/stories/:id/generate", Llm, 2),
    (Post, "/stories/:id/continue", Llm, 1),
    (Post, "/stories/:id/versions/:vid/spoken", Llm, 2),
    // ── Pitches ──────────────────────────────────────────────────
    (Post, "/pitches/:slug/turns", Llm, 1),
    (Post, "/pitches/:slug/generate", Llm, 2),
    (Post, "/pitches/:slug/continue", Llm, 1),
    (Post, "/pitches/:slug/versions/:vid/regenerate", Llm, 2),
    // ── Companies ────────────────────────────────────────────────
    // Creating a company kicks off the research agent synchronously;
    // refresh-packet is a manual re-run of the same agent.
    (Post, "/companies", Llm, 3),
    (Post, "/companies/:id/refresh-packet", Llm, 3),
    (Post, "/companies/:id/questions", Llm, 1),
    // ── Practice ─────────────────────────────────────────────────
    // Curator pool build can trigger an LLM call.
    (Post, "/practice", Llm, 1),
    // ── Assets ───────────────────────────────────────────────────
    // PDF upload + extraction is disk + CPU; charge 1 unit.
    (Post, "/assets", Upload, 1),
    (Post, "/assets/:id/reextract", Upload, 1),
    // ── Share / Fork ─────────────────────────────────────────────
    // Fork is one transaction copying a small JSONB and an N-row
    // question bank — no LLM, no audio. Publish/unpublish is a
    // single UPDATE. All three are billed at 0 so they show up
    // explicitly in the table (not in UNMETERED).
    (Post, "/browse/companies/:id/fork", Other, 0),
    (Post, "/companies/:id/publish", Other, 0),
    (Post, "/companies/:id/unpublish", Other, 0),
];

/// Look up the (kind, cost) for an in-flight request. Default is
/// `(PageNav, 0)` — handlers that don't appear in the table are treated as
/// free reads.
pub fn weight_for(method: &Method, path: &str) -> (RequestKind, u32) {
    for (m, template, kind, units) in WEIGHTS {
        if m.matches(method) && path_matches(template, path) {
            return (*kind, *units);
        }
    }
    (RequestKind::PageNav, 0)
}

/// True iff `template` matches `actual` segment-for-segment. Segments
/// beginning with `:` are wildcards that consume one non-empty segment;
/// `*name` consumes the rest. Anything else is a literal compare.
pub fn path_matches(template: &str, actual: &str) -> bool {
    let mut t = template.split('/');
    let mut a = actual.split('/');
    loop {
        match (t.next(), a.next()) {
            (None, None) => return true,
            (Some(ts), Some(as_)) => {
                if let Some(stripped) = ts.strip_prefix('*') {
                    return !stripped.is_empty() || !as_.is_empty();
                }
                if ts.starts_with(':') {
                    if as_.is_empty() {
                        return false;
                    }
                    continue;
                }
                if ts != as_ {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

/// Iterate every billed-entry template — used by the route-coverage test
/// in `tests/metering_route_coverage.rs`.
pub fn metered_templates() -> Vec<(Method, &'static str)> {
    WEIGHTS
        .iter()
        .map(|(m, t, _, _)| (m.as_axum(), *t))
        .collect()
}

/// Routes that are intentionally NOT metered. Reads, auth pages, static
/// fetches, and small CRUD that wouldn't make sense to count against a
/// daily request budget. Adding a route here is a billing decision — it
/// MUST be reviewed; the coverage test fails when a route exists in the
/// router but isn't in either this set OR `WEIGHTS`.
pub const UNMETERED: &[(HttpMethod, &str)] = &[
    // Auth + health + static + recordings — also `is_metering_exempt`'d by
    // the middleware. Listed here so the coverage test treats them uniformly.
    (Get, "/login"),
    (Post, "/login"),
    (Post, "/logout"),
    (Get, "/health"),
    (Get, "/recordings/*path"),
    // Pure reads / navigation.
    (Get, "/"),
    (Get, "/practice"),
    (Get, "/browse"),
    (Get, "/companies"),
    (Get, "/companies/new"),
    (Get, "/companies/:id"),
    (Get, "/stories"),
    (Get, "/stories/:id"),
    (Get, "/stories/:id/versions/:vid"),
    (Get, "/pitches"),
    (Get, "/pitches/:slug"),
    (Get, "/pitches/:slug/versions/:vid"),
    (Get, "/sessions/:id"),
    (Get, "/sessions/:id/review"),
    (Get, "/journal"),
    (Get, "/journal/new"),
    (Get, "/journal/:id"),
    (Get, "/journal/:id/edit"),
    (Get, "/assets"),
    (Get, "/assets/:id/preview"),
    (Get, "/settings"),
    (Get, "/agents"),
    (Get, "/agents/:name"),
    (Get, "/agents/:name/new"),
    (Get, "/agents/:name/history"),
    (Get, "/agents/:name/versions/:version_id"),
    (Get, "/categories"),
    // Small CRUD / state flips — no LLM, no audio. Each is one DB row
    // change; not worth charging against the daily budget.
    (Post, "/companies/:id/delete"),
    (Post, "/companies/:id/questions/:qid/delete"),
    (Post, "/companies/:id/questions/:qid/edit"),
    (Post, "/sessions/:id/toggle-voice"),
    (Post, "/sessions/:id/delete"),
    (Post, "/stories"),
    (Post, "/stories/:id/mode"),
    (Post, "/stories/:id/delete"),
    (Post, "/pitches/:slug/reset"),
    (Post, "/journal"),
    (Post, "/journal/:id/edit"),
    (Post, "/journal/:id/archive"),
    (Post, "/assets/:id/primary"),
    (Post, "/assets/:id/delete"),
    (Post, "/categories"),
    (Post, "/categories/:id/edit"),
    (Post, "/categories/:id/delete"),
    (Post, "/settings"),
    (Post, "/settings/refresh-models"),
    // Admin: invite-code management. Each request is one DB UPDATE /
    // INSERT — no LLM, no audio. Not metered.
    (Get, "/settings/invites"),
    (Post, "/settings/invites"),
    (Post, "/settings/invites/:id/extend"),
    (Post, "/settings/invites/:id/revoke"),
    (Post, "/agents/:name"),
    (Post, "/agents/:name/restore/:version_id"),
];

/// Whole table of "every endpoint this app exposes". The coverage test
/// asserts the assembled router's route list matches this. Function form
/// instead of a const slice because the entries are heterogeneous — keeps
/// the source of truth in one file.
pub fn all_known_routes() -> Vec<(HttpMethod, &'static str)> {
    let mut v: Vec<(HttpMethod, &'static str)> =
        WEIGHTS.iter().map(|(m, t, _, _)| (*m, *t)).collect();
    v.extend(UNMETERED.iter().copied());
    v
}

#[cfg(test)]
#[path = "weights_tests.rs"]
mod tests;
