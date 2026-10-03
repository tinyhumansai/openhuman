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

/// The HTTP status of a `web_fetch` error result. Anchored to the start of the
/// text, so a status quoted later (a response excerpt, another tool's output)
/// is not read, and to the whole shape, so a bare `HTTP 403` or `403 Forbidden`
/// from some other source is not either.
pub(super) fn fetched_site_status(text: &str) -> Option<u16> {
    let rest = text.trim_start().strip_prefix("HTTP ")?;
    let code = rest
        .get(..3)
        .filter(|c| c.bytes().all(|b| b.is_ascii_digit()))?;
    let rest = rest[3..].strip_prefix(' ')?;
    let status: u16 = code.parse().ok().filter(|s| (400..=599).contains(s))?;
    let (_reason, after_from) = rest.lines().next()?.split_once(" from ")?;
    let (host, _guidance) = after_from.split_once(';')?;
    (!host.is_empty() && !host.contains(char::is_whitespace)).then_some(status)
}

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
    if tool != WEB_FETCH_TOOL {
        return None;
    }
    let status = fetched_site_status(error)?;
    tracing::debug!(
        status,
        "[tinyagents::mw] web_fetch site status — not a credential failure"
    );
    Some(match status {
        401 | 403 => Some(("site_refused", 2)),
        429 | 500..=599 => Some(("transient", 2)),
        _ => None,
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
