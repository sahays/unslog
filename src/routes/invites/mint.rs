//! `POST /settings/invites` — mint a new invite, render the one-shot
//! "save this code now" page.
//!
//! Tier is parsed off a closed `<select>` so `UserTier::Master` is
//! unreachable; the service layer also rejects it defensively. Days is
//! restricted to `{30,60,90}` end-to-end.

use askama::Template;
use axum::extract::State;
use axum::response::{Html, IntoResponse, Response};
use axum::Form;
use serde::Deserialize;

use crate::error::AppError;
#[allow(unused_imports)]
use crate::filters; // base.html sidebar uses custom filters.
use crate::models::UserTier;
use crate::services::invite_admin::{InviteAdminDeps, MintInput, PgInviteAdminDeps};
use crate::startup::AppState;

#[derive(Deserialize)]
pub struct MintForm {
    pub label: String,
    pub tier: String,
    pub days: u32,
    #[serde(default)]
    #[allow(dead_code)]
    pub csrf_token: String,
}

#[derive(Template)]
#[template(path = "settings/invites_minted.html")]
struct MintedTemplate {
    plain_code: String,
    code_hint: String,
    label: String,
    tier_label: String,
    invitee_id: String,
}

pub(super) async fn submit(
    State(state): State<AppState>,
    Form(form): Form<MintForm>,
) -> Result<Response, AppError> {
    let tier = parse_tier(&form.tier)?;
    let deps = PgInviteAdminDeps {
        pool: state.pool.clone(),
    };
    let minted = deps
        .mint(MintInput {
            label: form.label.clone(),
            tier,
            days: form.days,
        })
        .await?;

    let tmpl = MintedTemplate {
        plain_code: minted.plain_code,
        code_hint: minted.code_hint,
        label: form.label,
        tier_label: tier_human_label(tier).into(),
        invitee_id: minted.id,
    };
    let html = tmpl
        .render()
        .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(html).into_response())
}

/// Map the form's tier string to a typed `UserTier`. Anything outside
/// the {pro, pro_max} closed set is a BadRequest — `master` is
/// explicitly rejected so the admin can't accidentally mint a second
/// master.
fn parse_tier(raw: &str) -> Result<UserTier, AppError> {
    match raw {
        "pro" => Ok(UserTier::Pro),
        "pro_max" => Ok(UserTier::ProMax),
        _ => Err(AppError::BadRequest(format!(
            "tier must be pro or pro_max, got {raw:?}"
        ))),
    }
}

fn tier_human_label(tier: UserTier) -> &'static str {
    match tier {
        UserTier::Pro => "Pro",
        UserTier::ProMax => "Pro Max",
        UserTier::Master => "Master",
    }
}
