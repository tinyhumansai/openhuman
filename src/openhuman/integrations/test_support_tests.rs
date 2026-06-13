use super::*;

async fn get_json(client: &reqwest::Client, url: String) -> Value {
    client
        .get(url)
        .send()
        .await
        .expect("GET request")
        .json::<Value>()
        .await
        .expect("GET json body")
}

async fn post_json(client: &reqwest::Client, url: String, body: Value) -> Value {
    client
        .post(url)
        .json(&body)
        .send()
        .await
        .expect("POST request")
        .json::<Value>()
        .await
        .expect("POST json body")
}

#[test]
fn as_string_array_filters_non_strings_and_missing_keys() {
    let value = json!({
        "searchQueries": ["alpha", 42, null, "beta"]
    });
    assert_eq!(
        as_string_array(&value, "searchQueries"),
        vec!["alpha".to_string(), "beta".to_string()]
    );
    assert!(as_string_array(&value, "missing").is_empty());
}

#[tokio::test]
async fn fake_backend_records_requests_and_applies_route_defaults() {
    let backend = spawn_fake_integration_backend().await;
    let client = reqwest::Client::new();

    let results = get_json(
        &client,
        format!(
            "{}/agent-integrations/apify/runs/run-123/results",
            backend.base_url
        ),
    )
    .await;
    assert_eq!(results["data"]["items"].as_array().unwrap().len(), 3);
    assert_eq!(results["data"]["total"], json!(8));

    let twilio = post_json(
        &client,
        format!("{}/agent-integrations/twilio/call", backend.base_url),
        json!({}),
    )
    .await;
    assert_eq!(twilio["data"]["callSid"], json!("CA0000"));

    let requests = backend.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].path,
        "/agent-integrations/apify/runs/run-123/results?limit=3&offset=0"
    );
    assert_eq!(requests[1].body, json!({}));
}

#[tokio::test]
async fn fake_backend_exercises_optional_and_fallback_response_paths() {
    let backend = spawn_fake_integration_backend().await;
    let client = reqwest::Client::new();

    let apify = post_json(
        &client,
        format!("{}/agent-integrations/apify/run", backend.base_url),
        json!({
            "actorId": "demo/actor",
            "input": { "hello": "world" },
            "sync": false
        }),
    )
    .await;
    assert_eq!(apify["data"]["status"], json!("RUNNING"));
    assert_eq!(apify["data"]["datasetId"], json!("dataset-demo-actor"));

    let extract = post_json(
        &client,
        format!("{}/agent-integrations/parallel/extract", backend.base_url),
        json!({
            "urls": ["https://example.com/a"]
        }),
    )
    .await;
    assert_eq!(extract["data"]["results"][0]["full_content"], Value::Null);
    assert_eq!(
        extract["data"]["results"][0]["excerpts"][0],
        json!("Focused on ")
    );

    let options = post_json(
        &client,
        format!(
            "{}/agent-integrations/financial-apis/options",
            backend.base_url
        ),
        json!({
            "symbol": "MSFT"
        }),
    )
    .await;
    assert_eq!(options["data"]["symbol"], json!("MSFT"));
    assert_eq!(options["data"]["contracts"][0]["delta"], Value::Null);

    let search = post_json(
        &client,
        format!("{}/agent-integrations/parallel/search", backend.base_url),
        json!({
            "objective": "fallback objective"
        }),
    )
    .await;
    assert_eq!(search["data"]["searchId"], json!("search-0"));
    assert_eq!(search["data"]["results"], json!([]));

    let tinyfish = post_json(
        &client,
        format!("{}/agent-integrations/tinyfish/agent/run", backend.base_url),
        json!({
            "url": "https://example.com",
            "goal": "Return JSON"
        }),
    )
    .await;
    assert_eq!(tinyfish["data"]["status"], json!("COMPLETED"));
    assert_eq!(tinyfish["data"]["result"]["ok"], json!(true));
}

#[tokio::test]
async fn fake_backend_caps_google_results_and_echoes_dynamic_inputs() {
    let backend = spawn_fake_integration_backend().await;
    let client = reqwest::Client::new();

    let places = post_json(
        &client,
        format!(
            "{}/agent-integrations/google-places/search",
            backend.base_url
        ),
        json!({
            "query": "pizza",
            "maxResults": 9
        }),
    )
    .await;
    assert_eq!(places["data"]["results"].as_array().unwrap().len(), 3);
    assert_eq!(
        places["data"]["results"][2]["name"],
        json!("pizza Result 3")
    );

    let commodity = post_json(
        &client,
        format!(
            "{}/agent-integrations/financial-apis/commodity",
            backend.base_url
        ),
        json!({
            "commodity": "BRENT"
        }),
    )
    .await;
    assert_eq!(commodity["data"]["series"]["interval"], json!("daily"));

    let exchange = post_json(
        &client,
        format!(
            "{}/agent-integrations/financial-apis/exchange-rate",
            backend.base_url
        ),
        json!({}),
    )
    .await;
    assert_eq!(exchange["data"]["rate"]["fromCurrency"], json!("BTC"));
    assert_eq!(exchange["data"]["rate"]["toCurrency"], json!("USD"));
}
