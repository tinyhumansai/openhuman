//! Core binary update policy and cached release-check metadata.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMode {
    Auto,
    Prompt,
    Manual,
}

impl Default for UpdateMode {
    fn default() -> Self {
        Self::Prompt
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdateConfig {
    #[serde(default)]
    pub mode: UpdateMode,
    #[serde(default = "default_check_interval_hours")]
    pub check_interval_hours: u64,
    #[serde(default)]
    pub last_check_at: Option<String>,
    #[serde(default)]
    pub last_seen_version: Option<String>,
    #[serde(default)]
    pub last_result: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub last_etag: Option<String>,
    #[serde(default)]
    pub last_dismissed_version: Option<String>,
    #[serde(default)]
    pub last_seen_tag: Option<String>,
    #[serde(default)]
    pub last_seen_asset_name: Option<String>,
    #[serde(default)]
    pub last_seen_download_url: Option<String>,
    #[serde(default)]
    pub last_seen_release_url: Option<String>,
    #[serde(default)]
    pub last_seen_digest_sha256: Option<String>,
}

fn default_check_interval_hours() -> u64 {
    24
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            mode: UpdateMode::Prompt,
            check_interval_hours: default_check_interval_hours(),
            last_check_at: None,
            last_seen_version: None,
            last_result: None,
            last_error: None,
            last_etag: None,
            last_dismissed_version: None,
            last_seen_tag: None,
            last_seen_asset_name: None,
            last_seen_download_url: None,
            last_seen_release_url: None,
            last_seen_digest_sha256: None,
        }
    }
}
