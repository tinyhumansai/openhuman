//! Handler for the `tools_apify_linkedin_scrape` controller schema.

use serde_json::{json, Map, Value};

use crate::config::rpc as config_rpc;
use crate::core::all::ControllerFuture;
use crate::core::Outcome;

pub(super) fn handle_apify_linkedin_scrape(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let profile_url = params
            .get("profile_url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| "missing or empty `profile_url`".to_string())?;

        let config = config_rpc::load_config_with_timeout().await?;
        let client = crate::integrations::build_client(&config).ok_or_else(|| {
            "Apify scrape unavailable — no backend session token. Sign in first.".to_string()
        })?;

        let data = super::linkedin::scrape_linkedin_profile(&client, &profile_url)
            .await
            .map_err(|e| format!("Apify LinkedIn scrape failed: {e:#}"))?;

        let markdown = super::linkedin::render_profile_markdown(&profile_url, &data);

        let payload = json!({ "data": data, "markdown": markdown });
        let log = vec![format!(
            "tools.apify_linkedin_scrape: url={profile_url} markdown_chars={}",
            markdown.chars().count()
        )];
        Outcome::new(payload, log).into_cli_compatible_json()
    })
}
