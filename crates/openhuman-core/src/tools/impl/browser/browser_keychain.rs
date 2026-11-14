//! Origin-bound password placeholders for the browser's `fill` and `type`.
//!
//! An agent that must enter a site's password types the placeholder
//! `{{keychain:<site>}}` instead. The password is swapped in here, at the
//! browser boundary, just before the action goes to TinyComputer, and only
//! when the page it is typed into is on `<site>` (the host itself or a
//! subdomain, over https). A look-alike page gets nothing: its host does not
//! match, so the action is refused and no password is fetched.
//!
//! The password comes from `[browser] keychain_command`, an executable the
//! host configures (never the agent: the settings RPC cannot set it). It is
//! run without a shell as `<command> <page host>` and its standard output is
//! the password. The password never reaches the model: the action the agent
//! sees, the approval gate's summary and every log line carry the
//! placeholder, and the reply TinyComputer sends back has the password
//! replaced before the tool returns it.

use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use serde_json::Value;
use tinycomputer_bus::browser::{Action, ReadRequest, SessionId};
use zeroize::Zeroizing;

use crate::modules::browser::BrowserClient;

const OPEN: &str = "{{keychain:";
const CLOSE: &str = "}}";
/// How long the keychain command may take.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
/// What a password the command prints may be at most.
const MAX_SECRET_BYTES: usize = 1024;
/// What stands in for a password in anything returned to the agent.
pub(super) const MASK: &str = "[keychain]";

/// How long a filled password is still masked in what the browser returns.
const REMEMBER_FILLED: Duration = Duration::from_secs(6 * 3600);
/// The most filled passwords remembered at once.
const MAX_REMEMBERED: usize = 64;

/// Passwords typed from the keychain lately, process-wide. A filled field
/// stays on the page: a later snapshot, page read or task step of any
/// session would show what was typed into a text field, so every browser
/// reply is masked against these, not only the fill's own.
fn filled() -> &'static std::sync::Mutex<Vec<(std::time::Instant, Zeroizing<String>)>> {
    static FILLED: std::sync::OnceLock<
        std::sync::Mutex<Vec<(std::time::Instant, Zeroizing<String>)>>,
    > = std::sync::OnceLock::new();
    FILLED.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

fn remember(secret: &Zeroizing<String>) {
    let Ok(mut list) = filled().lock() else {
        return;
    };
    let now = std::time::Instant::now();
    list.retain(|(at, _)| now.duration_since(*at) < REMEMBER_FILLED);
    if list
        .iter()
        .any(|(_, known)| known.as_str() == secret.as_str())
    {
        return;
    }
    if list.len() >= MAX_REMEMBERED {
        list.remove(0);
    }
    list.push((now, secret.clone()));
}

/// `text` with every password typed from the keychain lately masked.
pub(super) fn redact_filled(text: &str) -> String {
    let Ok(list) = filled().lock() else {
        return text.to_owned();
    };
    let mut out = text.to_owned();
    for (_, secret) in list.iter() {
        if !secret.is_empty() && out.contains(secret.as_str()) {
            out = redact(&out, secret);
        }
    }
    out
}

/// The site a value names when the whole value is one placeholder.
pub(super) fn placeholder_site(value: &str) -> Option<String> {
    let inner = value.trim().strip_prefix(OPEN)?.strip_suffix(CLOSE)?;
    let site = inner.trim().trim_end_matches('.').to_ascii_lowercase();
    let valid = !site.is_empty()
        && site.len() <= 253
        && site.contains('.')
        && site.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    valid.then_some(site)
}

/// Whether any part of a value looks like a placeholder: one that is not the
/// whole value (`pre{{keychain:x}}`) is refused rather than typed literally.
fn mentions_placeholder(value: &str) -> bool {
    value.contains(OPEN)
}

/// The page's host when `page_url` is an https page on `site` or under it.
pub(super) fn host_on_site(page_url: &str, site: &str) -> Option<String> {
    let url = url::Url::parse(page_url).ok()?;
    if url.scheme() != "https" {
        return None;
    }
    let host = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    (host == site || host.ends_with(&format!(".{site}"))).then_some(host)
}

/// `text` with every occurrence of `secret` replaced by [`MASK`].
pub(super) fn redact(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        return text.to_owned();
    }
    text.replace(secret, MASK)
}

/// A reply with every string that holds `secret` masked.
pub(super) fn redact_value(value: Value, secret: &str) -> Value {
    match value {
        Value::String(text) => Value::String(redact(&text, secret)),
        Value::Array(items) => {
            Value::Array(items.into_iter().map(|v| redact_value(v, secret)).collect())
        }
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, v)| (redact(&key, secret), redact_value(v, secret)))
                .collect(),
        ),
        other => other,
    }
}

/// The text field of a fill or type action.
fn typed_text(action: &mut Action) -> Option<&mut String> {
    match action {
        Action::Fill { value, .. } => Some(value),
        Action::Type { text, .. } => Some(text),
        _ => None,
    }
}

/// Runs the keychain command for `host` and returns what it printed.
pub(super) async fn run_command(command: &str, host: &str) -> anyhow::Result<Zeroizing<String>> {
    if !std::path::Path::new(command).is_absolute() {
        bail!("[browser] keychain_command must be an absolute path");
    }
    let child = tokio::process::Command::new(command)
        .arg(host)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| "the keychain command could not start")?;
    let output = tokio::time::timeout(COMMAND_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| {
            anyhow!(
                "the keychain command took longer than {}s",
                COMMAND_TIMEOUT.as_secs()
            )
        })?
        .with_context(|| "the keychain command failed")?;
    let stdout = Zeroizing::new(output.stdout);
    if !output.status.success() {
        bail!("the keychain has no password for {host}");
    }
    if stdout.len() > MAX_SECRET_BYTES {
        bail!("the keychain command printed more than a password");
    }
    let text = std::str::from_utf8(&stdout)
        .map_err(|_| anyhow!("the keychain command printed no text"))?;
    let secret = Zeroizing::new(text.trim_end_matches(['\r', '\n']).to_owned());
    if secret.is_empty() || secret.contains(['\r', '\n']) {
        bail!("the keychain command printed no single password");
    }
    Ok(secret)
}

/// Swaps a `{{keychain:<site>}}` placeholder in a fill or type for the
/// password, when the session's page is on that site. Returns the password
/// that was typed in, for the caller to mask in the reply; `None` when the
/// action carries no placeholder.
pub(super) async fn substitute(
    client: &BrowserClient,
    session: &SessionId,
    action: &mut Action,
) -> anyhow::Result<Option<Zeroizing<String>>> {
    let Some(text) = typed_text(action) else {
        return Ok(None);
    };
    let Some(site) = placeholder_site(text) else {
        if mentions_placeholder(text) {
            bail!("a keychain placeholder must be the whole value: {OPEN}<site>{CLOSE}");
        }
        return Ok(None);
    };
    let command = client
        .config()
        .browser
        .keychain_command
        .clone()
        .filter(|command| !command.trim().is_empty())
        .ok_or_else(|| anyhow!("no keychain is configured ([browser] keychain_command)"))?;
    let page = client
        .read_page(
            session,
            ReadRequest {
                max_chars: 1,
                ..ReadRequest::default()
            },
        )
        .await?;
    let Some(host) = host_on_site(&page.url, &site) else {
        tracing::warn!(site = %site, "[browser-keychain] refused: the page is not on the placeholder's site");
        bail!("the page is not on {site}; the keychain fills a password only on its own site");
    };
    let secret = run_command(&command, &host).await?;
    remember(&secret);
    tracing::info!(site = %site, "[browser-keychain] filled a password from the keychain");
    *text = secret.to_string();
    Ok(Some(secret))
}

#[cfg(test)]
#[path = "browser_keychain_tests.rs"]
mod tests;
