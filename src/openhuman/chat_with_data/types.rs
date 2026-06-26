//! Domain types for chat-with-data analytics assistant.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DataSource {
    Csv,
    Json,
    Sqlite,
    Api,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InsightType {
    Anomaly,
    Trend,
    Summary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRef {
    pub dataset: String,
    pub columns_used: Vec<String>,
    pub filter_applied: Option<String>,
    pub row_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub answer: String,
    pub sources: Vec<SourceRef>,
    pub confidence: f64,
    pub caveats: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Insight {
    pub id: String,
    pub insight_type: InsightType,
    pub title: String,
    pub description: String,
    pub dataset: String,
    pub severity: f64,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetMeta {
    pub id: String,
    pub name: String,
    pub source: DataSource,
    pub columns: Vec<String>,
    pub row_count: u64,
    pub registered_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_serializes() {
        assert_eq!(serde_json::to_string(&DataSource::Csv).unwrap(), "\"csv\"");
    }
    #[test]
    fn insight_type_serializes() {
        assert_eq!(
            serde_json::to_string(&InsightType::Anomaly).unwrap(),
            "\"anomaly\""
        );
    }
    #[test]
    fn query_result_round_trips() {
        let qr = QueryResult {
            answer: "42".into(),
            sources: vec![],
            confidence: 0.9,
            caveats: vec![],
        };
        let j = serde_json::to_string(&qr).unwrap();
        let back: QueryResult = serde_json::from_str(&j).unwrap();
        assert_eq!(back.answer, "42");
    }
}
