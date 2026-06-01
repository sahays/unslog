//! Weight-table coverage test.
//!
//! Walks every `.route("/...", get(..).post(..))` declaration under
//! `src/routes/` and asserts the (method, template) pair is either
//! billed in `weights::WEIGHTS` or explicitly allow-listed in
//! `weights::UNMETERED`. The point is to make adding a new endpoint
//! without a billing decision a compile/test-time failure rather than a
//! silently-unmetered route.
//!
//! Approach — parse the source under `src/routes/` directly. Axum 0.7
//! doesn't expose a route-iteration API, and maintaining a parallel
//! "list of routes" struct would just duplicate the canonical
//! `Router::new().route(..)` calls that already live in those files.

use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use unslog::services::metering::weights::{all_known_routes, path_matches, HttpMethod, UNMETERED};

#[test]
fn case_every_router_route_has_a_billing_decision() {
    let routes_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/routes");
    let declared = scan_routes(&routes_dir);
    assert!(
        !declared.is_empty(),
        "no .route() calls found under src/routes — scanner regex broken?",
    );

    let known: HashSet<(HttpMethod, String)> = all_known_routes()
        .into_iter()
        .map(|(m, t)| (m, t.to_string()))
        .collect();

    let mut missing: Vec<(HttpMethod, String)> = Vec::new();
    for (method, template) in &declared {
        if !known.contains(&(*method, template.clone())) {
            missing.push((*method, template.clone()));
        }
    }
    assert!(
        missing.is_empty(),
        "routes declared in src/routes/ have no billing decision in \
         services::metering::weights::WEIGHTS or UNMETERED — add an entry \
         (cost units or explicit unmetered): {missing:#?}",
    );
}

#[test]
fn case_unmetered_routes_have_zero_weight() {
    use axum::http::Method;
    for (m, template) in UNMETERED {
        let method: Method = m.as_axum();
        // Construct an example actual path by replacing :name segments
        // with a literal value so weight_for() resolves through path_matches.
        let actual = template
            .split('/')
            .map(|seg| {
                if seg.starts_with(':') {
                    "x"
                } else if seg.starts_with('*') {
                    "any/path/here"
                } else {
                    seg
                }
            })
            .collect::<Vec<_>>()
            .join("/");
        let (_, units) = unslog::services::metering::weights::weight_for(&method, &actual);
        assert_eq!(
            units, 0,
            "UNMETERED route {m:?} {template} resolved to {units} units — \
             move it out of UNMETERED or remove the WEIGHTS entry",
        );
    }
}

#[test]
fn case_known_route_templates_are_path_matches_self() {
    // Sanity: every template in our union resolves to itself under
    // path_matches(). Catches typos like missing leading `/`.
    for (_, t) in all_known_routes() {
        assert!(
            path_matches(t, &substitute_segments(t)),
            "template {t} fails to match its own substituted form",
        );
    }
}

fn substitute_segments(template: &str) -> String {
    template
        .split('/')
        .map(|s| {
            if s.starts_with(':') {
                "x"
            } else if s.starts_with('*') {
                "any/path"
            } else {
                s
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Recursively scan `dir` for `.route("...", ...)` declarations and
/// extract every (method, template) pair. Handles `get(..).post(..)`
/// chains and multi-line `.route(\n "...", \n post(..))` forms.
fn scan_routes(dir: &Path) -> Vec<(HttpMethod, String)> {
    // `.route("/path/here", get(handler).post(handler2))` and the
    // multi-line variant. The verbs we care about are the lowercase
    // axum::routing fns: get / post / put / patch / delete.
    let route_re = Regex::new(r#"\.route\(\s*"([^"]+)"\s*,\s*((?:[a-z]+\([^)]*\)\s*\.?)+)\s*\)"#)
        .expect("route regex");
    let verb_re = Regex::new(r"\b(get|post|put|patch|delete)\s*\(").expect("verb regex");

    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(entries) = fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let Some(ext) = p.extension() else { continue };
            if ext != "rs" {
                continue;
            }
            // Skip test files — they often declare scratch routers with
            // throw-away paths.
            if p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with("_tests.rs"))
                .unwrap_or(false)
            {
                continue;
            }
            let Ok(src) = fs::read_to_string(&p) else {
                continue;
            };
            for cap in route_re.captures_iter(&src) {
                let template = cap.get(1).unwrap().as_str().to_string();
                let verbs_block = cap.get(2).unwrap().as_str();
                for vcap in verb_re.captures_iter(verbs_block) {
                    let method = parse_verb(vcap.get(1).unwrap().as_str());
                    out.push((method, template.clone()));
                }
            }
        }
    }
    out
}

fn parse_verb(s: &str) -> HttpMethod {
    match s {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patch" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
        other => panic!("unexpected verb {other}"),
    }
}
