//! The Apify LinkedIn profile scrape behind `tools_apify_linkedin_scrape`:
//! run the `dev_fusion/linkedin-profile-scraper` actor through the backend's
//! Apify proxy and render the first profile item as Markdown.

use std::sync::Arc;

use serde_json::json;

use crate::integrations::IntegrationClient;

/// Apify actor slug for the LinkedIn profile scraper.
const LINKEDIN_SCRAPER_ACTOR: &str = "dev_fusion/linkedin-profile-scraper";

/// Call the Apify LinkedIn profile scraper synchronously and return the
/// first profile item from the dataset.
pub async fn scrape_linkedin_profile(
    client: &Arc<IntegrationClient>,
    profile_url: &str,
) -> anyhow::Result<serde_json::Value> {
    let body = json!({
        "actorId": LINKEDIN_SCRAPER_ACTOR,
        "input": {
            "profileUrls": [profile_url],
        },
        "sync": true,
        "timeoutSecs": 120,
    });

    tracing::debug!(
        actor = LINKEDIN_SCRAPER_ACTOR,
        url_len = profile_url.len(),
        "[apify:linkedin] invoking Apify actor"
    );

    // The backend wraps the Apify response in its standard envelope.
    // `IntegrationClient::post` already unwraps `{ success, data }`.
    let resp: serde_json::Value = client
        .post("/agent-integrations/apify/run", &body)
        .await
        .map_err(|e| anyhow::anyhow!("Apify run failed: {e:#}"))?;

    let status = resp
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("UNKNOWN");

    if status != "SUCCEEDED" {
        anyhow::bail!("Apify run finished with status: {status}");
    }

    // Extract the first item from the inline results array.
    let items = resp
        .get("items")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("Apify run returned no items array"))?;

    items
        .first()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Apify run returned an empty items array"))
}

/// Turn the Apify scrape JSON into clean Markdown.
pub fn render_profile_markdown(url: &str, data: &serde_json::Value) -> String {
    let s = |key: &str| {
        data.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };

    let full_name = s("fullName");
    let headline = s("headline");
    let location = s("addressWithCountry");
    let about = s("about");
    let connections = data.get("connections").and_then(|v| v.as_u64());
    let followers = data.get("followers").and_then(|v| v.as_u64());

    let mut md = format!("# User Profile — {full_name}\n\n");

    if !headline.is_empty() {
        md.push_str(&format!("**{headline}**\n\n"));
    }
    if !location.is_empty() {
        md.push_str(&format!("Location: {location}\n\n"));
    }
    md.push_str(&format!("LinkedIn: {url}\n\n"));
    if let (Some(c), Some(f)) = (connections, followers) {
        md.push_str(&format!("Connections: {c} | Followers: {f}\n\n"));
    }

    if !about.is_empty() {
        md.push_str("## About\n\n");
        md.push_str(&about);
        md.push_str("\n\n");
    }

    // Experience
    if let Some(exps) = data.get("experiences").and_then(|v| v.as_array()) {
        if !exps.is_empty() {
            md.push_str("## Experience\n\n");
            for exp in exps {
                let title = exp.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let company = exp.get("subtitle").and_then(|v| v.as_str()).unwrap_or("");
                let duration = exp.get("duration").and_then(|v| v.as_str()).unwrap_or("");
                let caption = exp.get("caption").and_then(|v| v.as_str()).unwrap_or("");
                let desc = exp
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                md.push_str(&format!("- **{title}**"));
                if !company.is_empty() {
                    md.push_str(&format!(" at {company}"));
                }
                if !duration.is_empty() {
                    md.push_str(&format!(" ({duration})"));
                }
                if !caption.is_empty() {
                    md.push_str(&format!(" — {caption}"));
                }
                md.push('\n');
                if !desc.is_empty() {
                    md.push_str(&format!("  {desc}\n"));
                }
            }
            md.push('\n');
        }
    }

    // Education
    if let Some(edus) = data.get("educations").and_then(|v| v.as_array()) {
        if !edus.is_empty() {
            md.push_str("## Education\n\n");
            for edu in edus {
                let school = edu.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let degree = edu.get("subtitle").and_then(|v| v.as_str()).unwrap_or("");
                md.push_str(&format!("- **{school}**"));
                if !degree.is_empty() {
                    md.push_str(&format!(" — {degree}"));
                }
                md.push('\n');
            }
            md.push('\n');
        }
    }

    // Languages
    if let Some(langs) = data.get("languages").and_then(|v| v.as_array()) {
        if !langs.is_empty() {
            let names: Vec<&str> = langs
                .iter()
                .filter_map(|l| l.get("name").and_then(|v| v.as_str()))
                .collect();
            if !names.is_empty() {
                md.push_str(&format!("Languages: {}\n\n", names.join(", ")));
            }
        }
    }

    // Volunteering
    if let Some(vols) = data.get("volunteering").and_then(|v| v.as_array()) {
        if !vols.is_empty() {
            md.push_str("## Volunteering\n\n");
            for vol in vols {
                let title = vol.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let org = vol.get("subtitle").and_then(|v| v.as_str()).unwrap_or("");
                md.push_str(&format!("- {title}"));
                if !org.is_empty() {
                    md.push_str(&format!(" at {org}"));
                }
                md.push('\n');
            }
            md.push('\n');
        }
    }

    md
}

#[cfg(test)]
#[path = "linkedin_tests.rs"]
mod tests;
