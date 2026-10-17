//! Attachments a host hands a turn as bytes, rather than as `[IMAGE:…]` /
//! `[FILE:…]` markers inside the message.
//!
//! A host that already holds a file's bytes (a channel webhook that
//! downloaded an inbound photo, an embedder with an upload) has nothing to
//! point a marker at, and a marker in user text is exactly what untrusted
//! input must not be able to forge. These are staged through the same
//! [`save`] as a marker's source, under the same count and size limits, and
//! become the same compact `[ATTACHMENT:…]` references, so conversion and the
//! provider treat them like any other upload. No local path is read, so they
//! are accepted for every turn origin.

use std::path::Path;

use tinyagents_harness::multimodal::mime::detect_file_mime;

use super::{save, AttachmentAccessScope};
use crate::config::Config;

/// One file a host attaches to a turn.
#[derive(Clone, PartialEq, Eq)]
pub struct HostAttachment {
    /// The file's contents.
    pub bytes: Vec<u8>,
    /// The MIME type the host was given for it. Blank or generic
    /// (`application/octet-stream`) types are sniffed from the name and bytes.
    pub mime_type: String,
    /// The file's name, when the host knows it. Only its sanitized last
    /// component is kept.
    pub name: Option<String>,
}

impl HostAttachment {
    /// An attachment of `bytes` typed `mime_type`, with no name.
    pub fn new(bytes: impl Into<Vec<u8>>, mime_type: impl Into<String>) -> Self {
        Self {
            bytes: bytes.into(),
            mime_type: mime_type.into(),
            name: None,
        }
    }

    /// The same attachment, named `name`.
    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

// Hand-written so a log line never carries the payload.
impl std::fmt::Debug for HostAttachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostAttachment")
            .field("mime_type", &self.mime_type)
            .field("size_bytes", &self.bytes.len())
            .field("named", &self.name.is_some())
            .finish()
    }
}

/// The MIME type `attachment` is staged as: what the host said, unless that
/// is blank or generic and the name or bytes say more.
fn mime_of(attachment: &HostAttachment) -> String {
    let declared = attachment.mime_type.trim();
    let name = attachment.name.as_deref().map(Path::new);
    // The detector trusts any header it knows, octet-stream included, so a
    // generic one is withheld for the name and bytes to decide.
    let header = (!declared.is_empty()
        && !declared.eq_ignore_ascii_case("application/octet-stream"))
    .then_some(declared);
    detect_file_mime(name, &attachment.bytes, header)
        .or_else(|| header.map(str::to_ascii_lowercase))
        .unwrap_or_else(|| "application/octet-stream".into())
}

/// Save every attachment for `thread` and return their references, in order,
/// one per line.
///
/// # Errors
///
/// Fails, staging nothing further, when the attachments exceed the configured
/// image or file count, one exceeds its size limit, an empty file is given,
/// or a file cannot be written.
pub(crate) async fn stage_host(
    attachments: &[HostAttachment],
    thread: &str,
    config: &Config,
    scope: &AttachmentAccessScope,
) -> anyhow::Result<String> {
    let (max_images, image_mb) = config.multimodal.effective_limits();
    let (max_files, file_mb, _) = config.multimodal_files.effective_limits();
    let typed: Vec<(&HostAttachment, String)> = attachments
        .iter()
        .map(|attachment| (attachment, mime_of(attachment)))
        .collect();
    let images = typed
        .iter()
        .filter(|(_, mime)| mime.starts_with("image/"))
        .count();
    if images > max_images || typed.len() - images > max_files {
        anyhow::bail!("attachment count exceeds configured limit");
    }
    let mut references = String::new();
    for (attachment, mime) in typed {
        let limit_mb = if mime.starts_with("image/") {
            image_mb
        } else {
            file_mb
        };
        if attachment.bytes.is_empty() {
            anyhow::bail!("attachment is empty");
        }
        if attachment.bytes.len() > limit_mb * 1024 * 1024 {
            anyhow::bail!("attachment exceeds configured size limit");
        }
        let name = attachment.name.as_deref().unwrap_or("attachment");
        let staged = save(config, thread, name, &mime, &attachment.bytes, scope).await?;
        log::debug!(
            "[attachments] staged host attachment mime={} size_bytes={}",
            staged.mime,
            staged.size_bytes
        );
        references.push_str(&staged.marker());
        references.push('\n');
    }
    Ok(references)
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
