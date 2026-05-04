use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedirectLink {
    pub id: String,
    pub url: String,
    pub short_url: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub hit_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewriteReplacement {
    pub original: String,
    pub replacement: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewriteResult {
    pub text: String,
    pub replacements: Vec<RewriteReplacement>,
}
