//! A public website's HTTP status, as `web_fetch` reports it (tinytools#47).
//!
//! `web_fetch` renders a 4xx/5xx from the site it fetched as an error result
//! shaped `HTTP <code> <reason> from <host>; <guidance>`, followed by an
//! excerpt of the response body. The site refusing a request says nothing about
//! OpenHuman's own credentials, but the keyword classifier
//! (`tools::status::classify`) reads `403`/`forbidden` as bad credentials, a
//! zero-retry class that would pause the whole run on one bot-blocked page.
//! The breaker therefore reads this shape itself, for `web_fetch` only, and
//! never sniffs the quoted body.

/// The agent tool that fetches a public URL (`tinytools` `web_fetch`).
pub(super) const WEB_FETCH_TOOL: &str = "web_fetch";

/// Recovery policy for a `web_fetch` error carrying a fetched site's status:
/// `None` when `tool`/`error` are not that shape (the caller classifies as
/// usual), otherwise the policy to return, itself `None` for an ordinary
/// failure.
///
/// - 401/403: the site refused us. Another page or source may still work, so
///   the run continues, but the same host refusing repeatedly stops it.
/// - 429 and 5xx: transient, with the usual retry headroom.
/// - anything else (404, 410, 400, ...): an ordinary tool failure with no
///   classified budget; the exact-repeat guard bounds it.
pub(super) fn fetched_site_policy(
    tool: &str,
    error: &str,
) -> Option<Option<(&'static str, usize)>> {
    use tinytools::status::FetchedSiteRecovery;
    let failure = tinytools::status::classify_web_fetch_status(tool, error, false)?;
    tracing::debug!(
        status = failure.status,
        "[tinyagents::mw] web_fetch site status — not a credential failure"
    );
    Some(match failure.recovery {
        FetchedSiteRecovery::SiteRefused => Some(("site_refused", 2)),
        FetchedSiteRecovery::Transient => Some(("transient", 2)),
        FetchedSiteRecovery::Ordinary => None,
    })
}

/// The failure-scope part for a `web_fetch` `url` argument: its host. A site
/// decides whether to serve us per host, not per page, so a model walking a
/// blocked site's pages shares one budget instead of a fresh one per URL.
pub(super) fn fetch_host_scope(tool: &str, field: &str, value: &str) -> Option<String> {
    if tool != WEB_FETCH_TOOL || field != "url" {
        return None;
    }
    let url = url::Url::parse(value).ok()?;
    Some(format!(":host={}", url.host_str()?))
}

/// The text the keyword heuristics may read for a failed `tool` result. For an
/// ordinary fetched-site status (404, 410, ...) the response body is untrusted
/// excerpt text, so only the status line is read; it must not turn a site
/// failure into a terminal inference or transient tool failure. Everything else
/// is read whole.
pub(super) fn heuristic_text<'a>(tool: &str, failure_text: &'a str) -> &'a str {
    match tinytools::status::classify_web_fetch_status(tool, failure_text, false) {
        Some(failure) if failure.recovery == tinytools::status::FetchedSiteRecovery::Ordinary => {
            failure.headline(failure_text)
        }
        _ => failure_text,
    }
}
