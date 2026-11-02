//! Read-only session state lookups.

use serde_json::json;

use crate::config::Config;
use crate::core::Outcome;
use crate::credentials::session_state::build_session_state;
use crate::credentials::session_state::get_session_token;

pub async fn auth_get_state(
    config: &Config,
) -> Result<Outcome<super::super::responses::AuthStateResponse>, String> {
    let state = build_session_state(config).await?;
    Ok(Outcome::single_log(state, "session state fetched"))
}

pub async fn auth_get_session_token_json(
    config: &Config,
) -> Result<Outcome<serde_json::Value>, String> {
    let token = get_session_token(config).await?;
    Ok(Outcome::single_log(
        json!({ "token": token }),
        "session token fetched",
    ))
}
