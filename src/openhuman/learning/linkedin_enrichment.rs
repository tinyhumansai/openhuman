//! LinkedIn profile enrichment via Gmail email mining + Apify scraping.
//!
//! Pipeline:
//!
//! 1. Search Gmail (via Composio) for emails from `linkedin.com`.
//! 2. Extract a `linkedin.com/in/<slug>` profile URL from the results.
//! 3. Scrape the profile via the Apify actor `dev_fusion/linkedin-profile-scraper`.
//! 4. Persist the scraped profile data into the user-profile memory namespace.
//!
//! Designed to run once during onboarding as a fire-and-forget enrichment
//! pass. Each stage logs progress so the caller (or a future frontend
//! progress UI) can observe what happened.

use crate::openhuman::config::Config;
use crate::openhuman::integrations::{build_client, IntegrationClient};
use regex::Regex;
use serde_json::json;
use std::sync::{Arc, LazyLock};

/// Apify actor slug for the LinkedIn profile scraper.
const LINKEDIN_SCRAPER_ACTOR: &str = "dev_fusion/linkedin-profile-scraper";

/// Regex that captures a LinkedIn username from profile URLs.
///
/// Matches both the canonical form (`linkedin.com/in/<slug>`) and the
/// notification-email form (`linkedin.com/comm/in/<slug>`). The username
/// is captured in group 1 so we can reconstruct a clean canonical URL.
static LINKEDIN_USERNAME_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?:www\.)?linkedin\.com/(?:comm/)?in/([a-zA-Z0-9_-]+)").unwrap()
});

/// Build the canonical profile URL from a username slug.
fn canonical_linkedin_url(username: &str) -> String {
    format!("https://www.linkedin.com/in/{username}")
}

/// Outcome of the full enrichment pipeline.
#[derive(Debug)]
pub struct LinkedInEnrichmentResult {
    /// The LinkedIn profile URL found in Gmail, if any.
    pub profile_url: Option<String>,
    /// Raw scraped profile JSON from Apify, if the scrape succeeded.
    pub profile_data: Option<serde_json::Value>,
    /// Human-readable summary of what happened at each stage.
    pub log: Vec<String>,
}

/// Run the full Gmail → LinkedIn �� Apify enrichment pipeline.
///
/// Returns `Ok` with a result struct even if individual stages fail —
/// partial progress is still useful. Only returns `Err` if we can't
/// even build the integration client (i.e. user isn't signed in).
pub async fn run_linkedin_enrichment(config: &Config) -> anyhow::Result<LinkedInEnrichmentResult> {
    let client = build_client(config)
        .ok_or_else(|| anyhow::anyhow!("no integration client — user not signed in"))?;

    let mut result = LinkedInEnrichmentResult {
        profile_url: None,
        profile_data: None,
        log: Vec::new(),
    };

    // ── Stage 1: search Gmail for LinkedIn emails ��───────────────────
    tracing::info!("[linkedin_enrichment] stage 1: searching Gmail for LinkedIn emails");
    result.log.push("Searching Gmail for LinkedIn emails...".into());

    let profile_url = match search_gmail_for_linkedin(config).await {
        Ok(Some(url)) => {
            tracing::info!(url = %url, "[linkedin_enrichment] found LinkedIn profile URL");
            result.log.push(format!("Found LinkedIn profile: {url}"));
            Some(url)
        }
        Ok(None) => {
            tracing::info!("[linkedin_enrichment] no LinkedIn profile URL found in emails");
            result.log.push("No LinkedIn profile URL found in emails.".into());
            None
        }
        Err(e) => {
            tracing::warn!(error = %e, "[linkedin_enrichment] Gmail search failed");
            result.log.push(format!("Gmail search failed: {e}"));
            None
        }
    };

    result.profile_url = profile_url.clone();

    // ── Stage 2: scrape the LinkedIn profile via Apify ───────────────
    let Some(url) = profile_url else {
        result.log.push("Skipping LinkedIn scrape — no profile URL.".into());
        return Ok(result);
    };

    tracing::info!(url = %url, "[linkedin_enrichment] stage 2: scraping LinkedIn profile via Apify");
    result.log.push("Scraping LinkedIn profile...".into());

    match scrape_linkedin_profile(&client, &url).await {
        Ok(data) => {
            tracing::info!("[linkedin_enrichment] Apify scrape succeeded");
            result.log.push("LinkedIn profile scraped successfully.".into());

            // ── Stage 3: write PROFILE.md to workspace ──────────────
            tracing::info!("[linkedin_enrichment] stage 3: writing PROFILE.md");
            if let Err(e) = write_profile_md(config, &url, &data).await {
                tracing::warn!(error = %e, "[linkedin_enrichment] failed to write PROFILE.md");
                result.log.push(format!("Failed to write PROFILE.md: {e}"));
            } else {
                result.log.push("PROFILE.md written to workspace.".into());
            }

            // Also persist to memory store for RAG retrieval.
            if let Err(e) = persist_linkedin_profile(config, &url, &data).await {
                tracing::warn!(error = %e, "[linkedin_enrichment] failed to persist to memory");
            }

            result.profile_data = Some(data);
        }
        Err(e) => {
            tracing::warn!(error = %e, "[linkedin_enrichment] Apify scrape failed");
            result.log.push(format!("LinkedIn scrape failed: {e}"));

            // Still write a minimal PROFILE.md with just the URL.
            if let Err(e) = write_profile_md_url_only(config, &url) {
                tracing::warn!(error = %e, "[linkedin_enrichment] failed to write PROFILE.md");
            }
            let _ = persist_linkedin_url_only(config, &url).await;
        }
    }

    Ok(result)
}

// ── PROFILE.md generation ────────────────────────────────────────────

/// Summarise the scraped LinkedIn data with an LLM, then write the
/// result to `{workspace_dir}/PROFILE.md`. The prompt system picks this
/// file up automatically on the next agent turn.
async fn write_profile_md(
    config: &Config,
    url: &str,
    data: &serde_json::Value,
) -> anyhow::Result<()> {
    // First render a full Markdown draft from the raw data.
    let raw_md = render_profile_markdown(url, data);

    // Then compress it through the LLM.
    let md = match summarise_profile_with_llm(config, &raw_md).await {
        Ok(summary) => summary,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "[linkedin_enrichment] LLM summarisation failed, falling back to raw markdown"
            );
            raw_md
        }
    };

    let path = config.workspace_dir.join("PROFILE.md");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &md)?;
    tracing::info!(path = %path.display(), len = md.len(), "[linkedin_enrichment] wrote PROFILE.md");
    Ok(())
}

/// Ask the backend LLM to distil the raw LinkedIn Markdown into a
/// concise, high-signal profile document suitable for agent context.
async fn summarise_profile_with_llm(config: &Config, raw_md: &str) -> anyhow::Result<String> {
    use crate::api::jwt::get_session_token;
    use crate::openhuman::providers::ops::{
        create_backend_inference_provider, ProviderRuntimeOptions,
    };

    let token = get_session_token(config)
        .map_err(|e| anyhow::anyhow!("failed to read session token: {e}"))?
        .ok_or_else(|| anyhow::anyhow!("no session token for LLM call"))?;

    let provider = create_backend_inference_provider(
        Some(&token),
        config.api_url.as_deref(),
        &ProviderRuntimeOptions::default(),
    )?;

    let system = "\
You are a profile analyst. You will receive a user's LinkedIn profile in Markdown format. \
Your job is to produce a concise PROFILE.md that an AI assistant will read to understand \
who this user is.\n\n\
Rules:\n\
- Output clean Markdown with a `# User Profile` heading.\n\
- Lead with name, headline, location, and LinkedIn URL.\n\
- Summarise the About section in 2-3 sentences max.\n\
- List only the most notable experiences (founder roles, leadership positions) — skip \
  short stints and minor roles.\n\
- Include education, languages, and any standout achievements.\n\
- Add a short `## Key facts for the assistant` section with 5-8 bullet points the AI \
  should know (e.g. expertise areas, industries, current focus, communication style hints).\n\
- Keep the entire output under 400 words.\n\
- Do not invent information — only use what is in the input.";

    let model = config
        .default_model
        .as_deref()
        .unwrap_or("neocortex-preview");

    tracing::debug!(
        model = model,
        input_len = raw_md.len(),
        "[linkedin_enrichment] sending profile to LLM for summarisation"
    );

    let summary = provider
        .chat_with_system(Some(system), raw_md, model, 0.3)
        .await?;

    tracing::debug!(
        output_len = summary.len(),
        "[linkedin_enrichment] LLM summarisation complete"
    );

    Ok(summary)
}

/// Minimal fallback when the Apify scrape failed but we have the URL.
fn write_profile_md_url_only(config: &Config, url: &str) -> anyhow::Result<()> {
    let md = format!(
        "# User Profile\n\n\
         LinkedIn: {url}\n\n\
         _Full profile data was not available at onboarding time._\n"
    );
    let path = config.workspace_dir.join("PROFILE.md");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, md)?;
    Ok(())
}

/// Turn the Apify scrape JSON into clean Markdown.
fn render_profile_markdown(url: &str, data: &serde_json::Value) -> String {
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
                let desc = exp.get("description").and_then(|v| v.as_str()).unwrap_or("");
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

// ── Internal helpers ─────────────────────────────────────────────────

/// Search Gmail via Composio for emails from linkedin.com and extract
/// the user's own LinkedIn username.
///
/// LinkedIn notification emails embed `comm/in/<username>` links in the
/// **HTML body** — which Gmail returns as base64-encoded data inside
/// `payload.parts[].body.data`. We must decode those parts before
/// regex-matching; searching the raw JSON alone misses them.
async fn search_gmail_for_linkedin(config: &Config) -> anyhow::Result<Option<String>> {
    use crate::openhuman::composio::client::build_composio_client;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;

    let client = build_composio_client(config)
        .ok_or_else(|| anyhow::anyhow!("composio client unavailable"))?;

    // `comm/in/<username>` — LinkedIn's own notification emails always use
    // this form to refer to the email *recipient's* profile.
    static COMM_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"linkedin\.com/comm/in/([a-zA-Z0-9_-]+)").unwrap()
    });

    let resp = client
        .execute_tool(
            "GMAIL_FETCH_EMAILS",
            Some(json!({
                "query": "from:linkedin.com",
                "max_results": 10,
            })),
        )
        .await
        .map_err(|e| anyhow::anyhow!("GMAIL_FETCH_EMAILS failed: {e:#}"))?;

    if !resp.successful {
        let err = resp.error.unwrap_or_else(|| "unknown error".into());
        anyhow::bail!("GMAIL_FETCH_EMAILS error: {err}");
    }

    // Walk the messages, decode HTML parts, and search for profile URLs.
    let messages = resp
        .data
        .get("messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for msg in &messages {
        // Collect all text to search: plain messageText + decoded HTML parts.
        let mut searchable = String::new();

        // Plain text body (already decoded by Composio).
        if let Some(text) = msg.get("messageText").and_then(|v| v.as_str()) {
            searchable.push_str(text);
            searchable.push('\n');
        }

        // Decode base64 HTML parts from payload.parts[].body.data.
        if let Some(parts) = msg
            .pointer("/payload/parts")
            .and_then(|v| v.as_array())
        {
            for part in parts {
                let is_html = part
                    .get("mimeType")
                    .and_then(|v| v.as_str())
                    .map_or(false, |m| m.contains("html"));
                if !is_html {
                    continue;
                }
                if let Some(b64) = part.pointer("/body/data").and_then(|v| v.as_str()) {
                    if let Ok(bytes) = URL_SAFE_NO_PAD.decode(b64) {
                        if let Ok(html) = String::from_utf8(bytes) {
                            searchable.push_str(&html);
                            searchable.push('\n');
                        }
                    }
                }
            }
        }

        // Priority 1: comm/in/<username> — always the recipient's own profile.
        if let Some(caps) = COMM_RE.captures(&searchable) {
            let username = caps[1].to_string();
            let url = canonical_linkedin_url(&username);
            tracing::info!(
                username = %username,
                url = %url,
                "[linkedin_enrichment] found own username via comm/in/ in HTML body"
            );
            return Ok(Some(url));
        }

        // Priority 2: canonical /in/<username> (some notification types).
        if let Some(caps) = LINKEDIN_USERNAME_RE.captures(&searchable) {
            let username = caps[1].to_string();
            let url = canonical_linkedin_url(&username);
            tracing::info!(
                username = %username,
                url = %url,
                "[linkedin_enrichment] found username via /in/ in email body"
            );
            return Ok(Some(url));
        }
    }

    Ok(None)
}

/// Call the Apify LinkedIn profile scraper synchronously and return the
/// first profile item from the dataset.
async fn scrape_linkedin_profile(
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
        url = profile_url,
        "[linkedin_enrichment] invoking Apify actor"
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

/// Persist the full scraped LinkedIn profile to the user-profile memory
/// namespace so the agent has rich context about the user.
async fn persist_linkedin_profile(
    _config: &Config,
    url: &str,
    data: &serde_json::Value,
) -> anyhow::Result<()> {
    use crate::openhuman::memory::store::MemoryClient;

    let memory = MemoryClient::new_local()
        .map_err(|e| anyhow::anyhow!("memory client unavailable: {e}"))?;

    let content = format!(
        "LinkedIn profile for {url}:\n\n{}",
        serde_json::to_string_pretty(data).unwrap_or_else(|_| data.to_string())
    );

    memory
        .store_skill_sync(
            "user-profile",   // namespace skill_id
            "linkedin",       // integration_id
            &format!("LinkedIn profile: {url}"),
            &content,
            Some("onboarding-linkedin-enrichment".into()),
            Some(json!({
                "source": "apify-linkedin-scraper",
                "url": url,
                "actor": LINKEDIN_SCRAPER_ACTOR,
            })),
            Some("high".into()),
            None, // created_at
            None, // updated_at
            None, // document_id
        )
        .await
        .map_err(|e| anyhow::anyhow!("memory store failed: {e}"))
}

/// Fallback: persist just the LinkedIn URL when the full scrape fails.
async fn persist_linkedin_url_only(_config: &Config, url: &str) -> anyhow::Result<()> {
    use crate::openhuman::memory::store::MemoryClient;

    let memory = MemoryClient::new_local()
        .map_err(|e| anyhow::anyhow!("memory client unavailable: {e}"))?;

    memory
        .store_skill_sync(
            "user-profile",
            "linkedin",
            &format!("LinkedIn profile URL: {url}"),
            &format!("User LinkedIn profile: {url}"),
            Some("onboarding-linkedin-url".into()),
            Some(json!({ "source": "gmail-linkedin-extraction", "url": url })),
            Some("medium".into()),
            None, // created_at
            None, // updated_at
            None, // document_id
        )
        .await
        .map_err(|e| anyhow::anyhow!("memory store failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_username_from_canonical_url() {
        let text = "Check out https://www.linkedin.com/in/williamhgates for more";
        let caps = LINKEDIN_USERNAME_RE.captures(text).unwrap();
        assert_eq!(&caps[1], "williamhgates");
        assert_eq!(
            canonical_linkedin_url(&caps[1]),
            "https://www.linkedin.com/in/williamhgates"
        );
    }

    #[test]
    fn extracts_username_from_comm_url() {
        let text = "https://www.linkedin.com/comm/in/stevenenamakel?midToken=abc";
        let caps = LINKEDIN_USERNAME_RE.captures(text).unwrap();
        assert_eq!(&caps[1], "stevenenamakel");
        assert_eq!(
            canonical_linkedin_url(&caps[1]),
            "https://www.linkedin.com/in/stevenenamakel"
        );
    }

    #[test]
    fn extracts_username_from_http_variant() {
        let text = "See http://www.linkedin.com/in/jeannie-wyrick-b4760710a";
        let caps = LINKEDIN_USERNAME_RE.captures(text).unwrap();
        assert_eq!(&caps[1], "jeannie-wyrick-b4760710a");
    }

    #[test]
    fn skips_non_profile_linkedin_urls() {
        let text = "Visit https://www.linkedin.com/company/openai";
        assert!(LINKEDIN_USERNAME_RE.captures(text).is_none());
    }

    #[test]
    fn handles_no_match() {
        assert!(LINKEDIN_USERNAME_RE.captures("No LinkedIn here").is_none());
    }
}
