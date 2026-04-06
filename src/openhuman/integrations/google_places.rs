//! Google Places integration tools — location search and place details.
//!
//! **Scope**: All (agent loop + CLI/RPC).
//!
//! **Endpoints**:
//!   - `POST /agent-integrations/google-places/search`
//!   - `POST /agent-integrations/google-places/details`
//!
//! **Pricing** (fetched from backend):
//!   - Search: ~$0.01/request  (base $0.005 + markup)
//!   - Details: ~$0.01/request (base $0.005 + markup)
//!
//! The backend handles Google API keys, billing, and rate limiting.

use super::{IntegrationClient, ToolScope};
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

// ── Response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct SearchResponse {
    results: Vec<PlaceResult>,
    #[serde(rename = "costUsd", default)]
    cost_usd: f64,
}

#[derive(Debug, Deserialize)]
struct PlaceResult {
    #[serde(rename = "placeId", default)]
    place_id: String,
    #[serde(default)]
    name: String,
    #[serde(rename = "formattedAddress", default)]
    formatted_address: String,
    #[serde(default)]
    rating: Option<f64>,
    #[serde(rename = "userRatingCount", default)]
    user_rating_count: Option<u64>,
    #[serde(rename = "googleMapsUri", default)]
    google_maps_uri: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DetailsResponse {
    place: PlaceDetails,
    #[serde(rename = "costUsd", default)]
    cost_usd: f64,
}

#[derive(Debug, Deserialize)]
struct PlaceDetails {
    #[serde(rename = "placeId", default)]
    place_id: String,
    #[serde(default)]
    name: String,
    #[serde(rename = "formattedAddress", default)]
    formatted_address: String,
    #[serde(default)]
    rating: Option<f64>,
    #[serde(rename = "userRatingCount", default)]
    user_rating_count: Option<u64>,
    #[serde(rename = "googleMapsUri", default)]
    google_maps_uri: Option<String>,
    #[serde(rename = "websiteUri", default)]
    website_uri: Option<String>,
    #[serde(rename = "nationalPhoneNumber", default)]
    national_phone_number: Option<String>,
    #[serde(rename = "businessStatus", default)]
    business_status: Option<String>,
    #[serde(rename = "regularOpeningHours", default)]
    regular_opening_hours: Option<OpeningHours>,
}

#[derive(Debug, Deserialize)]
struct OpeningHours {
    #[serde(rename = "openNow", default)]
    open_now: Option<bool>,
    #[serde(rename = "weekdayDescriptions", default)]
    weekday_descriptions: Vec<String>,
}

// ── GooglePlacesSearchTool ──────────────────────────────────────────

/// Search for places and businesses by text query.
pub struct GooglePlacesSearchTool {
    client: Arc<IntegrationClient>,
}

impl GooglePlacesSearchTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }

    pub fn scope(&self) -> ToolScope {
        ToolScope::All
    }
}

#[async_trait]
impl Tool for GooglePlacesSearchTool {
    fn name(&self) -> &str {
        "google_places_search"
    }

    fn description(&self) -> &str {
        "Search for places, businesses, or points of interest using Google Places. \
         Returns names, addresses, ratings, and place IDs that can be used with \
         google_places_details for more information. Cost is per request, billed by the backend."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query (e.g. 'coffee shops near Times Square')"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results (1-20, default 10)",
                    "minimum": 1,
                    "maximum": 20,
                    "default": 10
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: query"))?;

        if query.trim().is_empty() {
            return Ok(ToolResult::error("Search query cannot be empty"));
        }

        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .clamp(1, 20);

        let body = json!({
            "query": query,
            "maxResults": max_results
        });

        tracing::info!("[google_places_search] query={:?}", query);

        match self
            .client
            .post::<SearchResponse>("/agent-integrations/google-places/search", &body)
            .await
        {
            Ok(resp) => {
                if resp.results.is_empty() {
                    return Ok(ToolResult::success(format!(
                        "No places found for: {}",
                        query
                    )));
                }

                let mut lines = vec![format!(
                    "Found {} place(s) for: {}",
                    resp.results.len(),
                    query
                )];

                for (i, place) in resp.results.iter().enumerate() {
                    lines.push(format!("\n{}. {}", i + 1, place.name));
                    lines.push(format!("   Address: {}", place.formatted_address));
                    if let Some(rating) = place.rating {
                        let count = place.user_rating_count.unwrap_or(0);
                        lines.push(format!("   Rating: {:.1}/5 ({} reviews)", rating, count));
                    }
                    lines.push(format!("   Place ID: {}", place.place_id));
                    if let Some(ref uri) = place.google_maps_uri {
                        lines.push(format!("   Maps: {}", uri));
                    }
                }

                lines.push(format!("\nCost: ${:.4}", resp.cost_usd));
                Ok(ToolResult::success(lines.join("\n")))
            }
            Err(e) => Ok(ToolResult::error(format!(
                "Google Places search failed: {e}"
            ))),
        }
    }
}

// ── GooglePlacesDetailsTool ─────────────────────────────────────────

/// Get detailed information about a specific place by place ID.
pub struct GooglePlacesDetailsTool {
    client: Arc<IntegrationClient>,
}

impl GooglePlacesDetailsTool {
    pub fn new(client: Arc<IntegrationClient>) -> Self {
        Self { client }
    }

    pub fn scope(&self) -> ToolScope {
        ToolScope::All
    }
}

#[async_trait]
impl Tool for GooglePlacesDetailsTool {
    fn name(&self) -> &str {
        "google_places_details"
    }

    fn description(&self) -> &str {
        "Get detailed information about a specific place including hours, phone number, \
         website, rating, and business status. Requires a place ID from google_places_search. \
         Cost is per request, billed by the backend."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "place_id": {
                    "type": "string",
                    "description": "Google Places ID (from google_places_search results)"
                }
            },
            "required": ["place_id"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let place_id = args
            .get("place_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: place_id"))?;

        if place_id.trim().is_empty() {
            return Ok(ToolResult::error("place_id cannot be empty"));
        }

        let body = json!({ "placeId": place_id });

        tracing::info!("[google_places_details] placeId={}", place_id);

        match self
            .client
            .post::<DetailsResponse>("/agent-integrations/google-places/details", &body)
            .await
        {
            Ok(resp) => {
                let p = &resp.place;
                let mut lines = vec![
                    format!("{}", p.name),
                    format!("Address: {}", p.formatted_address),
                ];

                if let Some(rating) = p.rating {
                    let count = p.user_rating_count.unwrap_or(0);
                    lines.push(format!("Rating: {:.1}/5 ({} reviews)", rating, count));
                }

                if let Some(ref status) = p.business_status {
                    lines.push(format!("Status: {}", status));
                }

                if let Some(ref phone) = p.national_phone_number {
                    lines.push(format!("Phone: {}", phone));
                }

                if let Some(ref website) = p.website_uri {
                    lines.push(format!("Website: {}", website));
                }

                if let Some(ref hours) = p.regular_opening_hours {
                    if let Some(open_now) = hours.open_now {
                        lines.push(format!("Open now: {}", if open_now { "Yes" } else { "No" }));
                    }
                    if !hours.weekday_descriptions.is_empty() {
                        lines.push("Hours:".to_string());
                        for desc in &hours.weekday_descriptions {
                            lines.push(format!("  {}", desc));
                        }
                    }
                }

                if let Some(ref uri) = p.google_maps_uri {
                    lines.push(format!("Maps: {}", uri));
                }

                lines.push(format!("Place ID: {}", p.place_id));
                lines.push(format!("\nCost: ${:.4}", resp.cost_usd));
                Ok(ToolResult::success(lines.join("\n")))
            }
            Err(e) => Ok(ToolResult::error(format!(
                "Google Places details failed: {e}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_client() -> Arc<IntegrationClient> {
        Arc::new(IntegrationClient::new("http://test".into(), "tok".into()))
    }

    // ── GooglePlacesSearchTool ──────────────────────────────────────

    #[test]
    fn search_tool_metadata() {
        let tool = GooglePlacesSearchTool::new(test_client());
        assert_eq!(tool.name(), "google_places_search");
        assert_eq!(tool.scope(), ToolScope::All);
        assert!(tool.description().contains("Search for places"));
    }

    #[test]
    fn search_schema_has_required_query() {
        let tool = GooglePlacesSearchTool::new(test_client());
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["query"].is_object());
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "query"));
    }

    #[tokio::test]
    async fn search_rejects_missing_query() {
        let tool = GooglePlacesSearchTool::new(test_client());
        assert!(tool.execute(json!({})).await.is_err());
    }

    #[tokio::test]
    async fn search_rejects_empty_query() {
        let tool = GooglePlacesSearchTool::new(test_client());
        let result = tool.execute(json!({"query": ""})).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("empty"));
    }

    #[test]
    fn search_response_deserializes() {
        let json = r#"{
            "results": [
                {
                    "placeId": "ChIJ123",
                    "name": "Test Cafe",
                    "formattedAddress": "123 Main St",
                    "rating": 4.5,
                    "userRatingCount": 100
                }
            ],
            "costUsd": 0.01
        }"#;
        let resp: SearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].name, "Test Cafe");
        assert!((resp.cost_usd - 0.01).abs() < f64::EPSILON);
    }

    // ── GooglePlacesDetailsTool ─────────────────────────────────────

    #[test]
    fn details_tool_metadata() {
        let tool = GooglePlacesDetailsTool::new(test_client());
        assert_eq!(tool.name(), "google_places_details");
        assert_eq!(tool.scope(), ToolScope::All);
        assert!(tool.description().contains("detailed information"));
    }

    #[test]
    fn details_schema_has_required_place_id() {
        let tool = GooglePlacesDetailsTool::new(test_client());
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "place_id"));
    }

    #[tokio::test]
    async fn details_rejects_missing_place_id() {
        let tool = GooglePlacesDetailsTool::new(test_client());
        assert!(tool.execute(json!({})).await.is_err());
    }

    #[tokio::test]
    async fn details_rejects_empty_place_id() {
        let tool = GooglePlacesDetailsTool::new(test_client());
        let result = tool.execute(json!({"place_id": ""})).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("empty"));
    }

    #[test]
    fn details_response_deserializes() {
        let json = r#"{
            "place": {
                "placeId": "ChIJ123",
                "name": "Test Cafe",
                "formattedAddress": "123 Main St",
                "rating": 4.5,
                "userRatingCount": 100,
                "websiteUri": "https://test.com",
                "nationalPhoneNumber": "+1 555-1234",
                "businessStatus": "OPERATIONAL",
                "regularOpeningHours": {
                    "openNow": true,
                    "weekdayDescriptions": ["Monday: 9 AM - 5 PM"]
                }
            },
            "costUsd": 0.01
        }"#;
        let resp: DetailsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.place.name, "Test Cafe");
        assert_eq!(resp.place.website_uri.as_deref(), Some("https://test.com"));
        assert!(resp.place.regular_opening_hours.unwrap().open_now.unwrap());
    }
}
