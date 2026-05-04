use anyhow::Result;
use regex::Regex;
use serde_json::{json, Value};
use std::sync::OnceLock;

use crate::openhuman::config::Config;
use crate::openhuman::redirect_links::store;
use crate::openhuman::redirect_links::types::{RedirectLink, RewriteReplacement, RewriteResult};
use crate::rpc::RpcOutcome;

/// URLs shorter than this are not worth rewriting — the `openhuman://link/<id>`
/// placeholder is ~24 bytes, so shortening below this just wastes work and
/// tokens. Callers may override via `rewrite_inbound_with_threshold`.
pub const DEFAULT_MIN_URL_LEN: usize = 80;

fn url_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Wider than the reference regex to catch common tracking-URL characters
    // (`#`, `:`, `+`, `@`, `~`, `!`, `,`, `;`). Trailing sentence punctuation
    // is stripped below so regular prose doesn't get mangled.
    RE.get_or_init(|| Regex::new(r#"https?://[\w\d./\?=%\-&#:+@~!,;]+"#).unwrap())
}

fn short_url_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"openhuman://link/([0-9a-f]+)").unwrap())
}

fn public_url_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Anchor on `https?://` and match the `openhm.xyz` domain specifically to
    // avoid lookalikes (evil-openhm.xyz) or mid-token matches. Capture optional
    // query and fragment as separate tail parts so callers can safely insert
    // `?u=` into the query without polluting the fragment.
    RE.get_or_init(|| {
        Regex::new(
            r#"https?://openhm\.xyz/[A-Za-z0-9_-]+(?:\?[\w\d./\?=%\-&:+@~!,;]*)?(?:#[\w\d./\?=%\-&:+@~!,;]*)?"#,
        )
        .unwrap()
    })
}

/// Strip trailing sentence punctuation (`.`, `,`, `;`, `:`, `!`) so that
/// "see https://example.com/path." doesn't capture the period.
fn trim_trailing_punct(s: &str) -> &str {
    s.trim_end_matches(|c: char| matches!(c, '.' | ',' | ';' | ':' | '!'))
}

/// Shorten a single URL, persisting it in the global store. Idempotent.
pub fn shorten_url(config: &Config, url: &str) -> Result<RedirectLink> {
    store::shorten(config, url)
}

/// Expand a previously-shortened id back to its full URL. Bumps hit count.
pub fn expand_link(config: &Config, id: &str) -> Result<Option<RedirectLink>> {
    store::expand(config, id)
}

/// Rewrite every long URL in `text` to `openhuman://link/<id>`, using the
/// default length threshold.
pub fn rewrite_inbound(config: &Config, text: &str) -> Result<RewriteResult> {
    rewrite_inbound_with_threshold(config, text, DEFAULT_MIN_URL_LEN)
}

pub fn rewrite_inbound_with_threshold(
    config: &Config,
    text: &str,
    min_len: usize,
) -> Result<RewriteResult> {
    let re = url_regex();
    let mut replacements: Vec<RewriteReplacement> = Vec::new();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;

    for m in re.find_iter(text) {
        out.push_str(&text[cursor..m.start()]);
        let raw = m.as_str();
        let url = trim_trailing_punct(raw);
        let trailing = &raw[url.len()..];

        if url.len() >= min_len {
            let link = store::shorten(config, url)?;
            out.push_str(&link.short_url);
            replacements.push(RewriteReplacement {
                original: url.to_string(),
                replacement: link.short_url,
                id: link.id,
            });
        } else {
            out.push_str(url);
        }
        out.push_str(trailing);
        cursor = m.end();
    }
    out.push_str(&text[cursor..]);

    Ok(RewriteResult {
        text: out,
        replacements,
    })
}

/// Replace every `openhuman://link/<id>` placeholder with its stored URL.
/// Unknown ids are left as-is so nothing silently disappears.
pub fn rewrite_outbound(config: &Config, text: &str) -> Result<RewriteResult> {
    let re = short_url_regex();
    let mut replacements: Vec<RewriteReplacement> = Vec::new();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;

    for caps in re.captures_iter(text) {
        let whole = caps.get(0).unwrap();
        let id = caps.get(1).unwrap().as_str();
        out.push_str(&text[cursor..whole.start()]);

        match store::expand(config, id)? {
            Some(link) => {
                out.push_str(&link.url);
                replacements.push(RewriteReplacement {
                    original: whole.as_str().to_string(),
                    replacement: link.url,
                    id: link.id,
                });
            }
            None => {
                out.push_str(whole.as_str());
            }
        }
        cursor = whole.end();
    }
    out.push_str(&text[cursor..]);

    Ok(RewriteResult {
        text: out,
        replacements,
    })
}

/// Convenience wrapper that runs `rewrite_outbound` and then appends the
/// `user_id` to any public `openhm.xyz` links in the result.
pub fn rewrite_outbound_for_user(
    config: &Config,
    text: &str,
    user_id: Option<&str>,
) -> Result<RewriteResult> {
    let mut result = rewrite_outbound(config, text)?;
    result.text = append_user_id_to_public_links(&result.text, user_id);
    Ok(result)
}

/// Append `?u=<user_id>` to every `openhm.xyz/<id>` URL in a string.
/// If `user_id` is `None`, the text is returned unchanged.
/// Idempotent: URLs already containing a `u=` query parameter are left alone.
pub fn append_user_id_to_public_links(text: &str, user_id: Option<&str>) -> String {
    let Some(user_id) = user_id else {
        return text.to_string();
    };

    let re = public_url_regex();
    let encoded_user_id = urlencoding::encode(user_id);
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;

    for m in re.find_iter(text) {
        out.push_str(&text[cursor..m.start()]);
        let raw = m.as_str();
        let url = trim_trailing_punct(raw);
        let trailing = &raw[url.len()..];

        // Split off any fragment (#…) so `?u=` lands in the query, not the fragment.
        let (base, fragment) = match url.split_once('#') {
            Some((b, f)) => (b, Some(f)),
            None => (url, None),
        };

        if !base.contains("?u=") && !base.contains("&u=") {
            let separator = if base.contains('?') { "&" } else { "?" };
            out.push_str(base);
            out.push_str(separator);
            out.push_str("u=");
            out.push_str(&encoded_user_id);
        } else {
            out.push_str(base);
        }
        if let Some(frag) = fragment {
            out.push('#');
            out.push_str(frag);
        }
        out.push_str(trailing);
        cursor = m.end();
    }
    out.push_str(&text[cursor..]);
    out
}

// ── RPC handlers ────────────────────────────────────────────────────────

pub async fn rl_shorten(config: &Config, url: &str) -> Result<RpcOutcome<RedirectLink>, String> {
    let link = store::shorten(config, url).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        link.clone(),
        format!(
            "[redirect_links][rpc][shorten] id={} short_url={} original_url_len={}",
            link.id,
            link.short_url,
            link.url.len()
        ),
    ))
}

pub async fn rl_expand(config: &Config, id: &str) -> Result<RpcOutcome<Value>, String> {
    match store::expand(config, id).map_err(|e| e.to_string())? {
        Some(link) => Ok(RpcOutcome::new(
            serde_json::to_value(&link).map_err(|e| e.to_string())?,
            vec![format!(
                "[redirect_links][rpc][expand] id={} hit_count={}",
                link.id, link.hit_count
            )],
        )),
        None => Err(format!("[redirect_links][rpc][expand] not found: id={id}")),
    }
}

pub async fn rl_list(config: &Config, limit: Option<usize>) -> Result<RpcOutcome<Value>, String> {
    let limit = limit.unwrap_or(50).clamp(1, 1_000);
    let links = store::list(config, limit).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::new(
        json!({ "links": links }),
        vec![format!("[redirect_links][rpc][list] count={}", links.len())],
    ))
}

pub async fn rl_remove(config: &Config, id: &str) -> Result<RpcOutcome<Value>, String> {
    let removed = store::remove(config, id).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::new(
        json!({ "id": id, "removed": removed }),
        vec![format!(
            "[redirect_links][rpc][remove] id={id} removed={removed}"
        )],
    ))
}

pub async fn rl_rewrite_inbound(
    config: &Config,
    text: &str,
    min_len: Option<usize>,
) -> Result<RpcOutcome<RewriteResult>, String> {
    let result =
        rewrite_inbound_with_threshold(config, text, min_len.unwrap_or(DEFAULT_MIN_URL_LEN))
            .map_err(|e| e.to_string())?;
    let count = result.replacements.len();
    Ok(RpcOutcome::single_log(
        result,
        format!("[redirect_links][rpc][rewrite_inbound] replaced={count}"),
    ))
}

pub async fn rl_rewrite_outbound(
    config: &Config,
    text: &str,
) -> Result<RpcOutcome<RewriteResult>, String> {
    let result = rewrite_outbound(config, text).map_err(|e| e.to_string())?;
    let count = result.replacements.len();
    Ok(RpcOutcome::single_log(
        result,
        format!("[redirect_links][rpc][rewrite_outbound] expanded={count}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::Config;
    use tempfile::TempDir;

    fn test_config(tmp: &TempDir) -> Config {
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().join("workspace");
        std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
        cfg
    }

    const LONG: &str =
        "https://www.trip.com/forward/middlepages/channel/openEdm.gif?bizData=eyJldmVudCI6Im9wZW4iLCJmaWxlSWQiOiJmaWxlX2EwOD";

    #[test]
    fn inbound_shortens_long_urls_and_preserves_surrounding_text() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let text = format!("click here: {LONG} thanks");
        let result = rewrite_inbound(&cfg, &text).unwrap();
        assert!(result.text.starts_with("click here: openhuman://link/"));
        assert!(result.text.ends_with(" thanks"));
        assert_eq!(result.replacements.len(), 1);
    }

    #[test]
    fn inbound_leaves_short_urls_untouched() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let text = "see https://a.co/x for more";
        let result = rewrite_inbound(&cfg, text).unwrap();
        assert_eq!(result.text, text);
        assert!(result.replacements.is_empty());
    }

    #[test]
    fn inbound_trims_trailing_sentence_punctuation() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let text = format!("open {LONG}.");
        let result = rewrite_inbound(&cfg, &text).unwrap();
        assert!(result.text.ends_with("."));
        // The stored URL must not carry the trailing period.
        let link = &result.replacements[0];
        assert!(!link.original.ends_with('.'));
    }

    #[test]
    fn outbound_expands_placeholders_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let text = format!("go: {LONG}");
        let inbound = rewrite_inbound(&cfg, &text).unwrap();
        let outbound = rewrite_outbound(&cfg, &inbound.text).unwrap();
        assert_eq!(outbound.text, text);
    }

    #[test]
    fn outbound_leaves_unknown_ids_unchanged() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let text = "no match: openhuman://link/ffffffff";
        let result = rewrite_outbound(&cfg, text).unwrap();
        assert_eq!(result.text, text);
        assert!(result.replacements.is_empty());
    }

    #[test]
    fn inbound_handles_multiple_urls_in_one_string() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let text = format!("{LONG} and also {LONG}?extra=1234567890abcdef");
        let result = rewrite_inbound(&cfg, &text).unwrap();
        assert_eq!(result.replacements.len(), 2);
    }

    #[test]
    fn append_user_id_to_public_links_bare() {
        let text = "https://openhm.xyz/abc";
        let got = append_user_id_to_public_links(text, Some("nikhil"));
        assert_eq!(got, "https://openhm.xyz/abc?u=nikhil");
    }

    #[test]
    fn append_user_id_to_public_links_query() {
        let text = "https://openhm.xyz/abc?foo=bar";
        let got = append_user_id_to_public_links(text, Some("nikhil"));
        assert_eq!(got, "https://openhm.xyz/abc?foo=bar&u=nikhil");
    }

    #[test]
    fn append_user_id_to_public_links_idempotent() {
        // Already ?u=
        let text = "https://openhm.xyz/abc?u=existing";
        let got = append_user_id_to_public_links(text, Some("nikhil"));
        assert_eq!(got, text);

        // Already &u=
        let text = "https://openhm.xyz/abc?foo=bar&u=existing";
        let got = append_user_id_to_public_links(text, Some("nikhil"));
        assert_eq!(got, text);
    }

    #[test]
    fn append_user_id_to_public_links_none() {
        let text = "https://openhm.xyz/abc";
        let got = append_user_id_to_public_links(text, None);
        assert_eq!(got, text);
    }

    #[test]
    fn append_user_id_to_public_links_no_match() {
        let text = "https://example.com/abc";
        let got = append_user_id_to_public_links(text, Some("nikhil"));
        assert_eq!(got, text);
    }

    #[test]
    fn append_user_id_to_public_links_multiple() {
        let text = "https://openhm.xyz/a and https://openhm.xyz/b?x=y";
        let got = append_user_id_to_public_links(text, Some("nikhil"));
        assert_eq!(
            got,
            "https://openhm.xyz/a?u=nikhil and https://openhm.xyz/b?x=y&u=nikhil"
        );
    }

    #[test]
    fn append_user_id_to_public_links_encoding() {
        let text = "https://openhm.xyz/abc";
        let got = append_user_id_to_public_links(text, Some("nikhil@example.com + space"));
        assert_eq!(
            got,
            "https://openhm.xyz/abc?u=nikhil%40example.com%20%2B%20space"
        );
    }

    #[test]
    fn append_user_id_to_public_links_lookalikes() {
        let text = "https://evil-openhm.xyz/abc and openhm.xyz.evil.com/abc";
        let got = append_user_id_to_public_links(text, Some("nikhil"));
        assert_eq!(got, text);
    }

    #[test]
    fn append_user_id_to_public_links_punctuation() {
        let text = "Click https://openhm.xyz/abc.";
        let got = append_user_id_to_public_links(text, Some("nikhil"));
        assert_eq!(got, "Click https://openhm.xyz/abc?u=nikhil.");
    }

    #[test]
    fn append_user_id_to_public_links_query_with_fragment() {
        let text = "https://openhm.xyz/abc?foo=bar#frag";
        let got = append_user_id_to_public_links(text, Some("nikhil"));
        assert_eq!(got, "https://openhm.xyz/abc?foo=bar&u=nikhil#frag");
    }

    #[test]
    fn append_user_id_to_public_links_bare_with_fragment() {
        let text = "https://openhm.xyz/abc#frag";
        let got = append_user_id_to_public_links(text, Some("nikhil"));
        assert_eq!(got, "https://openhm.xyz/abc?u=nikhil#frag");
    }

    #[test]
    fn rewrite_outbound_for_user_expands_placeholder_and_tags_public_url() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);

        // Shorten LONG into an openhuman:// placeholder, then craft outbound text
        // that mixes the placeholder with a public openhm.xyz URL.
        let inbound = rewrite_inbound(&cfg, LONG).unwrap();
        let placeholder = &inbound.replacements[0].replacement;
        let text = format!("see {placeholder} and https://openhm.xyz/abc");

        let result = rewrite_outbound_for_user(&cfg, &text, Some("nikhil")).unwrap();
        assert!(
            result.text.contains(LONG),
            "placeholder must expand back to LONG"
        );
        assert!(
            result.text.contains("https://openhm.xyz/abc?u=nikhil"),
            "openhm.xyz URL must carry ?u= tag"
        );

        // None user_id leaves openhm.xyz untouched but still expands placeholder.
        let result_none = rewrite_outbound_for_user(&cfg, &text, None).unwrap();
        assert!(result_none.text.contains(LONG));
        assert!(result_none.text.contains("https://openhm.xyz/abc"));
        assert!(!result_none.text.contains("?u="));
    }
}
