//! Core types for memory sources.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Composio,
    Folder,
    GithubRepo,
    TwitterQuery,
    RssFeed,
    WebPage,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::Composio => "composio",
            SourceKind::Folder => "folder",
            SourceKind::GithubRepo => "github_repo",
            SourceKind::TwitterQuery => "twitter_query",
            SourceKind::RssFeed => "rss_feed",
            SourceKind::WebPage => "web_page",
        }
    }
}

/// A configured memory source entry persisted in `config.toml`.
///
/// All kind-specific fields are flattened onto the struct as `Option`s.
/// The `kind` discriminator determines which fields are required;
/// validation is enforced at add/update time via [`validate`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemorySourceEntry {
    pub id: String,
    pub kind: SourceKind,
    pub label: String,
    #[serde(default = "default_true")]
    pub enabled: bool,

    // ── Composio ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toolkit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,

    // ── Folder ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glob: Option<String>,

    // ── GithubRepo / RssFeed / WebPage / TwitterQuery (shared) ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    // ── GithubRepo ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,

    // ── TwitterQuery ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_days: Option<u32>,

    // ── RssFeed ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_items: Option<u32>,

    // ── WebPage ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
}

impl MemorySourceEntry {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() {
            return Err("id is required".to_string());
        }
        if self.label.is_empty() {
            return Err("label is required".to_string());
        }
        match self.kind {
            SourceKind::Composio => {
                require_field(&self.toolkit, "toolkit")?;
                require_field(&self.connection_id, "connection_id")?;
            }
            SourceKind::Folder => {
                require_field(&self.path, "path")?;
            }
            SourceKind::GithubRepo => {
                require_field(&self.url, "url")?;
            }
            SourceKind::TwitterQuery => {
                require_field(&self.query, "query")?;
            }
            SourceKind::RssFeed => {
                require_field(&self.url, "url")?;
            }
            SourceKind::WebPage => {
                require_field(&self.url, "url")?;
            }
        }
        Ok(())
    }
}

fn require_field(value: &Option<String>, name: &str) -> Result<(), String> {
    match value {
        Some(v) if !v.is_empty() => Ok(()),
        _ => Err(format!("{name} is required for this source kind")),
    }
}

/// One item listed from a source reader.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceItem {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentType {
    Markdown,
    Html,
    Plaintext,
}

/// Content read from a single source item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceContent {
    pub id: String,
    pub title: String,
    pub body: String,
    pub content_type: ContentType,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_kind_round_trips_via_serde() {
        for kind in [
            SourceKind::Composio,
            SourceKind::Folder,
            SourceKind::GithubRepo,
            SourceKind::TwitterQuery,
            SourceKind::RssFeed,
            SourceKind::WebPage,
        ] {
            let json = serde_json::to_string(&kind).unwrap();
            let decoded: SourceKind = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, kind);
        }
    }

    #[test]
    fn validate_composio_requires_toolkit_and_connection_id() {
        let entry = MemorySourceEntry {
            id: "src_1".into(),
            kind: SourceKind::Composio,
            label: "Gmail".into(),
            enabled: true,
            toolkit: Some("gmail".into()),
            connection_id: None,
            ..default_entry()
        };
        assert!(entry.validate().is_err());

        let valid = MemorySourceEntry {
            connection_id: Some("cmp_123".into()),
            ..entry
        };
        assert!(valid.validate().is_ok());
    }

    #[test]
    fn validate_folder_requires_path() {
        let entry = MemorySourceEntry {
            id: "src_2".into(),
            kind: SourceKind::Folder,
            label: "Notes".into(),
            enabled: true,
            path: None,
            ..default_entry()
        };
        assert!(entry.validate().is_err());
    }

    #[test]
    fn validate_github_requires_url() {
        let entry = MemorySourceEntry {
            id: "src_3".into(),
            kind: SourceKind::GithubRepo,
            label: "Repo".into(),
            enabled: true,
            url: Some("https://github.com/org/repo".into()),
            ..default_entry()
        };
        assert!(entry.validate().is_ok());
    }

    #[test]
    fn toml_round_trip() {
        let entry = MemorySourceEntry {
            id: "src_1".into(),
            kind: SourceKind::Folder,
            label: "My notes".into(),
            enabled: true,
            path: Some("/tmp/notes".into()),
            glob: Some("**/*.md".into()),
            ..default_entry()
        };
        let toml_str = toml::to_string_pretty(&entry).unwrap();
        let decoded: MemorySourceEntry = toml::from_str(&toml_str).unwrap();
        assert_eq!(decoded.id, "src_1");
        assert_eq!(decoded.kind, SourceKind::Folder);
        assert_eq!(decoded.path.as_deref(), Some("/tmp/notes"));
    }

    fn default_entry() -> MemorySourceEntry {
        MemorySourceEntry {
            id: String::new(),
            kind: SourceKind::Folder,
            label: String::new(),
            enabled: true,
            toolkit: None,
            connection_id: None,
            path: None,
            glob: None,
            url: None,
            branch: None,
            paths: Vec::new(),
            query: None,
            since_days: None,
            max_items: None,
            selector: None,
        }
    }
}
