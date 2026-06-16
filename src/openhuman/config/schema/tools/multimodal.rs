//! Multimodal (image + file) config types.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MultimodalConfig {
    #[serde(default = "default_multimodal_max_images")]
    pub max_images: usize,
    #[serde(default = "default_multimodal_max_image_size_mb")]
    pub max_image_size_mb: usize,
    #[serde(default)]
    pub allow_remote_fetch: bool,
}

fn default_multimodal_max_images() -> usize {
    4
}

fn default_multimodal_max_image_size_mb() -> usize {
    8
}

impl MultimodalConfig {
    /// Clamp configured values to safe runtime bounds.
    pub fn effective_limits(&self) -> (usize, usize) {
        let max_images = self.max_images.clamp(1, 16);
        let max_image_size_mb = self.max_image_size_mb.clamp(1, 20);
        (max_images, max_image_size_mb)
    }

    /// Clamp image count to the configured maximum.
    pub fn clamp_image_count(&self, count: usize) -> usize {
        count.min(self.max_images)
    }
}

impl Default for MultimodalConfig {
    fn default() -> Self {
        Self {
            max_images: default_multimodal_max_images(),
            max_image_size_mb: default_multimodal_max_image_size_mb(),
            allow_remote_fetch: false,
        }
    }
}

/// File-attachment counterpart to [`MultimodalConfig`]. Governs how
/// `[FILE:…]` markers in user messages are resolved, validated, and
/// inlined as text context for the agent.
///
/// Defaults err on the side of "useful for prose docs without blowing
/// the context window": 4 files per turn, 16 MB per file, 50 000 chars
/// of extracted text per file. Remote fetch is opt-in.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MultimodalFileConfig {
    #[serde(default = "default_multimodal_max_files")]
    pub max_files: usize,
    #[serde(default = "default_multimodal_max_file_size_mb")]
    pub max_file_size_mb: usize,
    #[serde(default = "default_multimodal_max_extracted_text_chars")]
    pub max_extracted_text_chars: usize,
    #[serde(default)]
    pub allow_remote_fetch: bool,
    #[serde(default = "default_multimodal_allowed_file_mime_types")]
    pub allowed_mime_types: Vec<String>,
}

fn default_multimodal_max_files() -> usize {
    4
}

fn default_multimodal_max_file_size_mb() -> usize {
    16
}

fn default_multimodal_max_extracted_text_chars() -> usize {
    50_000
}

fn default_multimodal_allowed_file_mime_types() -> Vec<String> {
    vec![
        // Extractable text formats.
        "application/pdf".to_string(),
        "text/plain".to_string(),
        "text/csv".to_string(),
        "text/markdown".to_string(),
        // Binary-only formats surfaced as metadata-only references.
        "application/zip".to_string(),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string(),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document".to_string(),
        "application/vnd.openxmlformats-officedocument.presentationml.presentation".to_string(),
        "application/octet-stream".to_string(),
    ]
}

impl MultimodalFileConfig {
    /// Clamp configured values to safe runtime bounds.
    pub fn effective_limits(&self) -> (usize, usize, usize) {
        let max_files = self.max_files.clamp(1, 16);
        let max_file_size_mb = self.max_file_size_mb.clamp(1, 50);
        let max_extracted_text_chars = self.max_extracted_text_chars.clamp(1_000, 200_000);
        (max_files, max_file_size_mb, max_extracted_text_chars)
    }

    /// True iff `mime` is on the configured allowlist (case-insensitive).
    pub fn is_mime_allowed(&self, mime: &str) -> bool {
        let needle = mime.to_ascii_lowercase();
        self.allowed_mime_types
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(&needle))
    }

    /// Hardened config for turns whose user text originates from an
    /// untrusted third-party channel (Slack / Discord / Telegram /
    /// WhatsApp / etc.). Disables `[FILE:…]` marker resolution outright
    /// so a remote sender cannot smuggle `[FILE:/etc/passwd]`,
    /// `[FILE:.env]`, or any other local-path marker into an inbound
    /// message and have the agent exfiltrate the file's contents into
    /// an LLM call. Also forbids remote fetch.
    ///
    /// `max_files: 0` is a sentinel: `prepare_messages_for_provider`
    /// short-circuits at the first `[FILE:…]` marker with
    /// `TooManyFiles` before any disk or network read happens. This
    /// holds regardless of the per-operator
    /// `[tools.multimodal_files]` block in `config.toml`.
    ///
    /// Mirrors the triage-arm hardening in
    /// `openhuman::agent::triage::evaluator`. Apply at the per-turn
    /// application site (the channel-runtime dispatcher) — the
    /// operator-supplied `config.multimodal_files` stays the source of
    /// truth for the desktop / web-chat path where the user owns the
    /// local filesystem.
    pub fn for_untrusted_channel_input() -> Self {
        Self {
            max_files: 0,
            allow_remote_fetch: false,
            ..Default::default()
        }
    }
}

impl Default for MultimodalFileConfig {
    fn default() -> Self {
        Self {
            max_files: default_multimodal_max_files(),
            max_file_size_mb: default_multimodal_max_file_size_mb(),
            max_extracted_text_chars: default_multimodal_max_extracted_text_chars(),
            allow_remote_fetch: false,
            allowed_mime_types: default_multimodal_allowed_file_mime_types(),
        }
    }
}
