use super::*;
use crate::openhuman::integrations::ToolScope;

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
