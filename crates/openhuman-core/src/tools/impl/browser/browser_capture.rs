//! Pictures of the page the browser tool takes by itself, for the host's
//! picture sinks (`modules::browser_pictures`).
//!
//! A host that wants to show what a browser run did (a post, a report) needs
//! a picture of the pages it reached. Leaving that to the model fails: the
//! tool has no screenshot action, and a model forgets. So while a sink is
//! registered the tool captures the page itself after it opens one, after a
//! click, key press, select or check, and before it closes a session, and
//! hands each picture to the sinks. The core keeps none of them.
//!
//! A capture never fails the call it follows: whatever goes wrong is logged
//! and the picture is skipped.
//!
//! Passwords: a password the keychain typed (`browser_keychain`) is masked
//! in every reply, but a picture cannot be masked. Before a capture, while
//! any keychain password is remembered, the page's text and field values
//! are read in the page ([`PAGE_TEXT_JS`], which leaves the refs the model
//! holds alone) and the capture is skipped when any shows one of those
//! passwords (a field switched to show it, an echo of it), or when the page
//! cannot be checked. A masked password field shows dots, not the password,
//! and is captured. A fill or type is never followed by a capture.

use tinycomputer_bus::browser::{ImageFormat, ScreenshotRequest, SessionId};

use crate::modules::browser::BrowserClient;
use crate::modules::browser_pictures::{self, BrowserPicture};

/// The largest picture handed over: X takes a still image up to 5 MB. A
/// larger PNG is taken again as a JPEG; a larger JPEG is skipped.
pub(crate) const MAX_PICTURE_BYTES: usize = 5 * 1024 * 1024;
/// JPEG quality for a page whose PNG is too large.
const JPEG_QUALITY: u8 = 80;

/// The direct actions a picture follows: the ones that change what the page
/// shows. Never `fill` or `type`, which leave typed text on the page.
const CAPTURED_AFTER: &[&str] = &["click", "double_click", "press", "select", "check"];

/// Whether a direct browser action of `kind` (its TinyComputer wire name) is
/// followed by a picture.
pub(super) fn follows(kind: &str) -> bool {
    CAPTURED_AFTER.contains(&kind)
}

/// What a picture of the page could show as text: its rendered text, the
/// values of its fields (a password field's own value is masked on screen
/// and left out; one switched to show its password is a text field), and
/// the same inside open shadow roots and same-origin frames. It goes to the
/// host's check only, never to a model or a log.
pub(super) const PAGE_TEXT_JS: &str = r#"(() => {
  const out = [];
  const visit = (root) => {
    if (!root) return;
    if (root.body && root.body.innerText) out.push(root.body.innerText);
    if (root instanceof ShadowRoot && root.textContent) out.push(root.textContent);
    for (const el of root.querySelectorAll('input, textarea, select, [contenteditable]')) {
      if (el.type === 'password') continue;
      if (typeof el.value === 'string' && el.value) out.push(el.value);
      else if (el.isContentEditable && el.textContent) out.push(el.textContent);
    }
    for (const el of root.querySelectorAll('*')) if (el.shadowRoot) visit(el.shadowRoot);
    for (const frame of root.querySelectorAll('iframe, frame')) {
      try { visit(frame.contentDocument); } catch (e) {}
    }
  };
  visit(document);
  return out;
})()"#;

/// The texts [`PAGE_TEXT_JS`] answered; `None` when the answer is not a
/// list of strings, which counts as a page that could not be checked.
pub(super) fn page_texts(value: &serde_json::Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|item| item.as_str().map(str::to_owned))
        .collect()
}

/// Whether any of `texts` shows a password the keychain typed lately.
pub(super) fn shows_filled_password(texts: &[&str]) -> bool {
    texts
        .iter()
        .any(|text| super::keychain::redact_filled(text) != *text)
}

/// Captures the session's page for the registered sinks, if there are any.
/// Never fails: a picture that cannot be taken is logged and skipped.
pub(super) async fn capture(client: &BrowserClient, session: &SessionId, after: &str) {
    if !browser_pictures::wanted() {
        return;
    }
    match take(client, session, after).await {
        Ok(Some(picture)) => {
            let took = browser_pictures::deliver(&picture);
            tracing::debug!(
                after,
                bytes = picture.bytes.len(),
                sinks = took,
                "[browser-capture] handed a picture of the page over"
            );
        }
        Ok(None) => {}
        Err(error) => tracing::warn!(
            after,
            error = %super::keychain::redact_filled(&error.to_string()),
            "[browser-capture] could not take a picture of the page"
        ),
    }
}

async fn take(
    client: &BrowserClient,
    session: &SessionId,
    after: &str,
) -> anyhow::Result<Option<BrowserPicture>> {
    if super::keychain::any_filled() {
        // Read in the page, never through a snapshot: a new snapshot would
        // renumber the refs the model is holding.
        let shown = client.evaluate(session, PAGE_TEXT_JS).await?;
        let Some(texts) = page_texts(&shown) else {
            tracing::info!(
                after,
                "[browser-capture] skipped: the page could not be checked for passwords"
            );
            return Ok(None);
        };
        let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
        if shows_filled_password(&texts) {
            tracing::info!(
                after,
                "[browser-capture] skipped: the page shows a password the keychain typed"
            );
            return Ok(None);
        }
    }
    let mut format = ImageFormat::Png;
    let mut bytes = client
        .screenshot(session, ScreenshotRequest::default())
        .await?;
    if bytes.len() > MAX_PICTURE_BYTES {
        format = ImageFormat::Jpeg;
        bytes = client
            .screenshot(
                session,
                ScreenshotRequest {
                    format,
                    quality: Some(JPEG_QUALITY),
                    ..ScreenshotRequest::default()
                },
            )
            .await?;
    }
    if bytes.len() > MAX_PICTURE_BYTES {
        tracing::info!(
            after,
            bytes = bytes.len(),
            "[browser-capture] skipped: the picture is over 5 MB even as a JPEG"
        );
        return Ok(None);
    }
    let taken_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    Ok(Some(BrowserPicture {
        bytes,
        media_type: format.media_type(),
        after: after.to_owned(),
        taken_at_ms,
    }))
}

#[cfg(test)]
#[path = "browser_capture_tests.rs"]
mod tests;
