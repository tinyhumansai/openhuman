use axum::{
    extract::{Path, Query},
    routing::{get, post},
    Json, Router,
};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub body: Value,
}

#[derive(Clone)]
struct FakeIntegrationState {
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

pub struct FakeIntegrationBackend {
    pub base_url: String,
    state: FakeIntegrationState,
}

impl FakeIntegrationBackend {
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.state.requests.lock().clone()
    }
}

fn record(state: &FakeIntegrationState, method: &str, path: String, body: Value) {
    state.requests.lock().push(RecordedRequest {
        method: method.to_string(),
        path,
        body,
    });
}

fn as_string_array(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub async fn spawn_fake_integration_backend() -> FakeIntegrationBackend {
    let state = FakeIntegrationState {
        requests: Arc::new(Mutex::new(Vec::new())),
    };

    let app = Router::new()
        .route(
            "/agent-integrations/apify/run",
            post({
                let state = state.clone();
                move |Json(body): Json<Value>| async move {
                    record(
                        &state,
                        "POST",
                        "/agent-integrations/apify/run".to_string(),
                        body.clone(),
                    );
                    let actor_id = body
                        .get("actorId")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown-actor");
                    let run_id = format!("run-{}", actor_id.replace('/', "-"));
                    let input = body.get("input").cloned().unwrap_or(Value::Null);
                    Json(json!({
                        "success": true,
                        "data": {
                            "runId": run_id,
                            "actorId": actor_id,
                            "status": if body.get("sync").and_then(Value::as_bool).unwrap_or(true) { "SUCCEEDED" } else { "RUNNING" },
                            "datasetId": format!("dataset-{}", actor_id.replace('/', "-")),
                            "items": [
                                {
                                    "actorId": actor_id,
                                    "inputEcho": input
                                }
                            ],
                            "costUsd": 0.31
                        }
                    }))
                }
            }),
        )
        .route(
            "/agent-integrations/apify/runs/{run_id}",
            get({
                let state = state.clone();
                move |Path(run_id): Path<String>| async move {
                    record(
                        &state,
                        "GET",
                        format!("/agent-integrations/apify/runs/{run_id}"),
                        Value::Null,
                    );
                    Json(json!({
                        "success": true,
                        "data": {
                            "runId": run_id,
                            "actorId": "apify/linkedin-profile-scraper",
                            "status": "SUCCEEDED",
                            "datasetId": format!("dataset-{run_id}"),
                            "costUsd": 0.02
                        }
                    }))
                }
            }),
        )
        .route(
            "/agent-integrations/apify/runs/{run_id}/results",
            get({
                let state = state.clone();
                move |Path(run_id): Path<String>,
                      Query(params): Query<HashMap<String, String>>| async move {
                    let limit = params
                        .get("limit")
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(3);
                    let offset = params
                        .get("offset")
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0);
                    record(
                        &state,
                        "GET",
                        format!(
                            "/agent-integrations/apify/runs/{run_id}/results?limit={limit}&offset={offset}"
                        ),
                        Value::Null,
                    );
                    let items: Vec<Value> = (0..limit)
                        .map(|idx| {
                            let n = offset + idx;
                            json!({
                                "runId": run_id,
                                "index": n,
                                "url": format!("https://example.com/{run_id}/{n}")
                            })
                        })
                        .collect();
                    Json(json!({
                        "success": true,
                        "data": {
                            "items": items,
                            "total": offset + limit + 5
                        }
                    }))
                }
            }),
        )
        .route(
            "/agent-integrations/google-places/search",
            post({
                let state = state.clone();
                move |Json(body): Json<Value>| async move {
                    record(
                        &state,
                        "POST",
                        "/agent-integrations/google-places/search".to_string(),
                        body.clone(),
                    );
                    let query = body.get("query").and_then(Value::as_str).unwrap_or("unknown");
                    let max_results =
                        body.get("maxResults").and_then(Value::as_u64).unwrap_or(10).min(3);
                    let results: Vec<Value> = (1..=max_results)
                        .map(|idx| {
                            json!({
                                "placeId": format!("place-{idx}-{query}"),
                                "name": format!("{query} Result {idx}"),
                                "formattedAddress": format!("{idx} {query} Street"),
                                "rating": 4.0 + (idx as f64 / 10.0),
                                "userRatingCount": idx * 111,
                                "googleMapsUri": format!("https://maps.google.com/?q={query}+{idx}")
                            })
                        })
                        .collect();
                    Json(json!({
                        "success": true,
                        "data": {
                            "results": results,
                            "costUsd": 0.01
                        }
                    }))
                }
            }),
        )
        .route(
            "/agent-integrations/google-places/details",
            post({
                let state = state.clone();
                move |Json(body): Json<Value>| async move {
                    record(
                        &state,
                        "POST",
                        "/agent-integrations/google-places/details".to_string(),
                        body.clone(),
                    );
                    let place_id = body.get("placeId").and_then(Value::as_str).unwrap_or("place");
                    Json(json!({
                        "success": true,
                        "data": {
                            "place": {
                                "placeId": place_id,
                                "name": format!("Details for {place_id}"),
                                "formattedAddress": format!("1 {place_id} Plaza"),
                                "rating": 4.8,
                                "userRatingCount": 321,
                                "googleMapsUri": format!("https://maps.google.com/?cid={place_id}"),
                                "websiteUri": format!("https://{place_id}.example.com"),
                                "nationalPhoneNumber": "+1 555-0100",
                                "businessStatus": "OPERATIONAL",
                                "regularOpeningHours": {
                                    "openNow": true,
                                    "weekdayDescriptions": ["Monday: 9:00 AM - 5:00 PM"]
                                }
                            },
                            "costUsd": 0.01
                        }
                    }))
                }
            }),
        )
        .route(
            "/agent-integrations/parallel/search",
            post({
                let state = state.clone();
                move |Json(body): Json<Value>| async move {
                    record(
                        &state,
                        "POST",
                        "/agent-integrations/parallel/search".to_string(),
                        body.clone(),
                    );
                    let objective = body
                        .get("objective")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown objective");
                    let queries = as_string_array(&body, "searchQueries");
                    let results: Vec<Value> = queries
                        .iter()
                        .enumerate()
                        .map(|(idx, query)| {
                            json!({
                                "url": format!("https://search.example.com/{idx}"),
                                "title": format!("Result for {query}"),
                                "publish_date": "2026-05-16",
                                "excerpts": [format!("Objective: {objective}; query: {query}")]
                            })
                        })
                        .collect();
                    Json(json!({
                        "success": true,
                        "data": {
                            "searchId": format!("search-{}", queries.len()),
                            "results": results,
                            "costUsd": 0.02
                        }
                    }))
                }
            }),
        )
        .route(
            "/agent-integrations/parallel/extract",
            post({
                let state = state.clone();
                move |Json(body): Json<Value>| async move {
                    record(
                        &state,
                        "POST",
                        "/agent-integrations/parallel/extract".to_string(),
                        body.clone(),
                    );
                    let urls = as_string_array(&body, "urls");
                    let objective = body.get("objective").and_then(Value::as_str).unwrap_or("");
                    let include_full = body
                        .get("fullContent")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let results: Vec<Value> = urls
                        .iter()
                        .map(|url| {
                            json!({
                                "url": url,
                                "title": format!("Extracted {url}"),
                                "excerpts": [format!("Focused on {objective}")],
                                "full_content": if include_full {
                                    Value::String(format!("Full content for {url}"))
                                } else {
                                    Value::Null
                                }
                            })
                        })
                        .collect();
                    Json(json!({
                        "success": true,
                        "data": {
                            "extractId": "extract-1",
                            "results": results,
                            "errors": [],
                            "costUsd": 0.03
                        }
                    }))
                }
            }),
        )
        .route(
            "/agent-integrations/parallel/chat",
            post({
                let state = state.clone();
                move |Json(body): Json<Value>| async move {
                    record(
                        &state,
                        "POST",
                        "/agent-integrations/parallel/chat".to_string(),
                        body.clone(),
                    );
                    let model = body.get("model").and_then(Value::as_str).unwrap_or("base");
                    let last_message = body
                        .get("messages")
                        .and_then(Value::as_array)
                        .and_then(|messages| messages.last())
                        .and_then(|message| message.get("content"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    Json(json!({
                        "success": true,
                        "data": {
                            "choices": [
                                {
                                    "message": {
                                        "role": "assistant",
                                        "content": format!("Model {model} answered: {last_message}")
                                    },
                                    "finish_reason": "stop"
                                }
                            ],
                            "basis": {
                                "sources": ["https://search.example.com/0"]
                            },
                            "costUsd": 0.04
                        }
                    }))
                }
            }),
        )
        .route(
            "/agent-integrations/parallel/research",
            post({
                let state = state.clone();
                move |Json(body): Json<Value>| async move {
                    record(
                        &state,
                        "POST",
                        "/agent-integrations/parallel/research".to_string(),
                        body.clone(),
                    );
                    let processor = body
                        .get("processor")
                        .and_then(Value::as_str)
                        .unwrap_or("base");
                    let input = body.get("input").cloned().unwrap_or(Value::Null);
                    Json(json!({
                        "success": true,
                        "data": {
                            "runId": format!("research-{processor}"),
                            "status": "completed",
                            "result": {
                                "processor": processor,
                                "inputEcho": input
                            },
                            "costUsd": 0.05
                        }
                    }))
                }
            }),
        )
        .route(
            "/agent-integrations/parallel/enrich",
            post({
                let state = state.clone();
                move |Json(body): Json<Value>| async move {
                    record(
                        &state,
                        "POST",
                        "/agent-integrations/parallel/enrich".to_string(),
                        body.clone(),
                    );
                    let input = body.get("input").cloned().unwrap_or(Value::Null);
                    Json(json!({
                        "success": true,
                        "data": {
                            "runId": "enrich-1",
                            "status": "completed",
                            "output": {
                                "inputEcho": input,
                                "summary": "Enriched entity"
                            },
                            "costUsd": 0.06
                        }
                    }))
                }
            }),
        )
        .route(
            "/agent-integrations/parallel/dataset",
            post({
                let state = state.clone();
                move |Json(body): Json<Value>| async move {
                    record(
                        &state,
                        "POST",
                        "/agent-integrations/parallel/dataset".to_string(),
                        body.clone(),
                    );
                    let entity_type = body
                        .get("entityType")
                        .and_then(Value::as_str)
                        .unwrap_or("entity");
                    let match_limit = body
                        .get("matchLimit")
                        .and_then(Value::as_u64)
                        .unwrap_or(10);
                    Json(json!({
                        "success": true,
                        "data": {
                            "findallId": format!("dataset-{entity_type}"),
                            "status": {
                                "state": "queued",
                                "entityType": entity_type
                            },
                            "matchLimit": match_limit,
                            "costUsd": 0.07
                        }
                    }))
                }
            }),
        )
        .route(
            "/agent-integrations/financial-apis/quote",
            post({
                let state = state.clone();
                move |Json(body): Json<Value>| async move {
                    record(
                        &state,
                        "POST",
                        "/agent-integrations/financial-apis/quote".to_string(),
                        body.clone(),
                    );
                    let symbol = body.get("symbol").and_then(Value::as_str).unwrap_or("AAPL");
                    let base = 100.0 + symbol.len() as f64;
                    Json(json!({
                        "success": true,
                        "data": {
                            "quote": {
                                "symbol": symbol,
                                "price": base + 1.25,
                                "open": base,
                                "high": base + 2.0,
                                "low": base - 1.0,
                                "volume": 123456.0,
                                "previousClose": base - 0.75,
                                "change": 2.0,
                                "changePercent": "1.95%",
                                "latestTradingDay": "2026-05-16"
                            },
                            "costUsd": 0.001
                        }
                    }))
                }
            }),
        )
        .route(
            "/agent-integrations/financial-apis/exchange-rate",
            post({
                let state = state.clone();
                move |Json(body): Json<Value>| async move {
                    record(
                        &state,
                        "POST",
                        "/agent-integrations/financial-apis/exchange-rate".to_string(),
                        body.clone(),
                    );
                    let from = body
                        .get("fromCurrency")
                        .and_then(Value::as_str)
                        .unwrap_or("BTC");
                    let to = body
                        .get("toCurrency")
                        .and_then(Value::as_str)
                        .unwrap_or("USD");
                    Json(json!({
                        "success": true,
                        "data": {
                            "rate": {
                                "fromCurrency": from,
                                "toCurrency": to,
                                "rate": 42.5,
                                "bid": 42.4,
                                "ask": 42.6,
                                "lastRefreshed": "2026-05-16 12:00:00",
                                "timeZone": "UTC"
                            },
                            "costUsd": 0.001
                        }
                    }))
                }
            }),
        )
        .route(
            "/agent-integrations/financial-apis/options",
            post({
                let state = state.clone();
                move |Json(body): Json<Value>| async move {
                    record(
                        &state,
                        "POST",
                        "/agent-integrations/financial-apis/options".to_string(),
                        body.clone(),
                    );
                    let symbol = body.get("symbol").and_then(Value::as_str).unwrap_or("AAPL");
                    let require_greeks = body
                        .get("requireGreeks")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    Json(json!({
                        "success": true,
                        "data": {
                            "symbol": symbol,
                            "contracts": [
                                {
                                    "type": "call",
                                    "expiration": "2026-06-19",
                                    "strike": "250",
                                    "last": "5.10",
                                    "bid": "5.00",
                                    "ask": "5.20",
                                    "delta": if require_greeks { json!("0.42") } else { Value::Null }
                                }
                            ],
                            "costUsd": 0.002
                        }
                    }))
                }
            }),
        )
        .route(
            "/agent-integrations/financial-apis/crypto-series",
            post({
                let state = state.clone();
                move |Json(body): Json<Value>| async move {
                    record(
                        &state,
                        "POST",
                        "/agent-integrations/financial-apis/crypto-series".to_string(),
                        body.clone(),
                    );
                    let symbol = body.get("symbol").and_then(Value::as_str).unwrap_or("BTC");
                    let market = body.get("market").and_then(Value::as_str).unwrap_or("USD");
                    Json(json!({
                        "success": true,
                        "data": {
                            "series": {
                                "symbol": symbol,
                                "market": market,
                                "series": [
                                    {
                                        "date": "2026-05-16",
                                        "open": 100.0,
                                        "high": 110.0,
                                        "low": 95.0,
                                        "close": 105.0,
                                        "volume": 1234.0
                                    },
                                    {
                                        "date": "2026-05-15",
                                        "open": 90.0,
                                        "high": 101.0,
                                        "low": 88.0,
                                        "close": 100.0,
                                        "volume": 1111.0
                                    }
                                ]
                            },
                            "costUsd": 0.003
                        }
                    }))
                }
            }),
        )
        .route(
            "/agent-integrations/financial-apis/commodity",
            post({
                let state = state.clone();
                move |Json(body): Json<Value>| async move {
                    record(
                        &state,
                        "POST",
                        "/agent-integrations/financial-apis/commodity".to_string(),
                        body.clone(),
                    );
                    let commodity = body
                        .get("commodity")
                        .and_then(Value::as_str)
                        .unwrap_or("WTI");
                    let interval = body
                        .get("interval")
                        .and_then(Value::as_str)
                        .unwrap_or("daily");
                    Json(json!({
                        "success": true,
                        "data": {
                            "series": {
                                "commodity": commodity,
                                "interval": interval,
                                "unit": "USD",
                                "series": [
                                    { "date": "2026-05-16", "value": 80.1 },
                                    { "date": "2026-05-15", "value": 79.8 }
                                ]
                            },
                            "costUsd": 0.004
                        }
                    }))
                }
            }),
        )
        .route(
            "/agent-integrations/twilio/call",
            post({
                let state = state.clone();
                move |Json(body): Json<Value>| async move {
                    record(
                        &state,
                        "POST",
                        "/agent-integrations/twilio/call".to_string(),
                        body.clone(),
                    );
                    let to = body.get("to").and_then(Value::as_str).unwrap_or("+10000000000");
                    let suffix: String = to.chars().filter(|c| c.is_ascii_digit()).rev().take(4).collect();
                    let suffix: String = suffix.chars().rev().collect();
                    Json(json!({
                        "success": true,
                        "data": {
                            "callSid": format!("CA{suffix}"),
                            "status": "queued",
                            "costUsd": 0.03
                        }
                    }))
                }
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fake integration backend");
    let addr = listener.local_addr().expect("fake backend local addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    FakeIntegrationBackend {
        base_url: format!("http://127.0.0.1:{}", addr.port()),
        state,
    }
}

#[cfg(test)]
#[path = "test_support_test.rs"]
mod tests;
