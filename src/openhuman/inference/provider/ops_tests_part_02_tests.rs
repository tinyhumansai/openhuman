use super::*;

#[tokio::test]
async fn list_models_empty_body_returns_diagnostic_error() {
    // Some misconfigured load balancers return 200 with an empty body.
    let tmp = tempfile::tempdir().expect("tempdir");
    let endpoint = spawn_static_models_server(StatusCode::OK, "").await;
    let config = configure_generic_workspace(&tmp, endpoint).await;

    let err = list_configured_models_from_config("generic-test", &config)
        .await
        .expect_err("empty body must not parse as JSON");

    assert!(
        err.contains("failed to parse JSON"),
        "error must keep canonical prefix: {err}"
    );
}

#[tokio::test]
async fn list_models_valid_json_still_succeeds() {
    // Regression guard: the new text-then-parse path must still accept
    // a valid `/models` JSON response.
    let tmp = tempfile::tempdir().expect("tempdir");
    let body = r#"{"data":[{"id":"some-model","owned_by":"vendor","context_length":4096}]}"#;
    let endpoint = spawn_static_models_server(StatusCode::OK, body).await;
    let config = configure_generic_workspace(&tmp, endpoint).await;

    let outcome = list_configured_models_from_config("generic-test", &config)
        .await
        .expect("valid JSON must list models");
    assert_eq!(outcome.value["models"][0]["id"], "some-model");
}

// ── parse_models_response (TAURI-RUST-4Y) ──────────────────────────────
//
// Before this fix the `/models` parser collapsed "no `data` field" and
// "`data` field present but not an array" into a single misleading
// error string: `"provider response missing `data` array — endpoint is
// not OpenAI-compatible (got keys: data, object)"` — the keys list
// included `data`, contradicting the "missing" claim. The split
// surfaces the actual JSON-type mismatch so future Sentry events on
// this code path are triageable instead of looking like the parser
// is hallucinating.

#[test]
fn parse_models_response_returns_models_for_well_formed_data_array() {
    // Happy path — exact OpenAI `/models` shape, must yield model ids
    // and `owned_by` / `context_length` projections from each entry.
    let body = serde_json::json!({
        "object": "list",
        "data": [
            { "id": "m1", "owned_by": "openai", "context_length": 8192 },
            { "id": "m2", "owned_by": "openai" },
            { "id": "m3", "context_window": 4096 },
        ],
    });
    let models = parse_models_response(&body).expect("well-formed body must parse");
    assert_eq!(models.len(), 3);
    assert_eq!(models[0].id, "m1");
    assert_eq!(models[0].owned_by.as_deref(), Some("openai"));
    assert_eq!(models[0].context_window, Some(8192));
    assert_eq!(models[2].id, "m3");
    assert_eq!(models[2].owned_by, None);
    assert_eq!(models[2].context_window, Some(4096));
}

#[test]
fn parse_models_response_returns_models_for_codex_models_array() {
    let body = serde_json::json!({
        "models": [
            { "slug": "gpt-5.5", "owned_by_organization": "openai", "max_context_window": 272000 },
            "gpt-5.4",
        ],
    });

    let models = parse_models_response(&body).expect("Codex models body must parse");

    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "gpt-5.5");
    assert_eq!(models[0].owned_by.as_deref(), Some("openai"));
    assert_eq!(models[0].context_window, Some(272000));
    assert_eq!(models[1].id, "gpt-5.4");
}

#[test]
fn parse_models_response_distinguishes_missing_data_field_from_wrong_type() {
    // (1) `data`/`models` fields completely absent — wrong endpoint
    // misconfiguration. Codex uses `models`, so it is accepted alongside
    // OpenAI-compatible `data`.
    let body = serde_json::json!({ "object": "list", "items": [] });
    let err = parse_models_response(&body).expect_err("no model catalog field must fail");
    assert!(
        err.contains("missing `data` or `models` field"),
        "no-data error should say `missing`: {err}"
    );
    assert!(
        err.contains("items") && err.contains("object"),
        "no-data error should list actual keys: {err}"
    );

    // (2) `data` field present but wrong type — TAURI-RUST-4Y verbatim
    // shape (`object` + `data` keys both present, but `data` isn't an
    // array). The error MUST NOT say "missing" — it must surface the
    // actual JSON type so triage knows what shape the provider sent.
    // `null` is deliberately excluded here — it is a valid empty catalog,
    // not a wrong type (see `parse_models_response_treats_null_data_as_empty_list`).
    for (label, value) in [
        (
            "object",
            serde_json::json!({"object":"error","message":"boom"}),
        ),
        ("string", serde_json::json!("models go here")),
        ("bool", serde_json::json!(true)),
        ("number", serde_json::json!(42)),
    ] {
        let body = serde_json::json!({ "object": "list", "data": value });
        let err = parse_models_response(&body).expect_err("wrong-type data must fail");
        assert!(
            !err.contains("missing"),
            "wrong-type error must not say `missing` ({label}): {err}"
        );
        assert!(
            err.contains(label),
            "wrong-type error must name the actual JSON kind ({label}): {err}"
        );
    }
}

#[test]
fn parse_models_response_treats_null_data_as_empty_list() {
    // TAURI-RUST-874 / TAURI-RUST-875: Ollama's OpenAI-compatible
    // `/v1/models` null-encodes the catalog (`{"object":"list","data":null}`)
    // when no models are pulled. A null `data`/`models` field is a valid empty
    // model list, not a malformed envelope — it MUST parse to an empty Vec
    // instead of manufacturing a hard error that floods Sentry.
    let data_null = serde_json::json!({ "object": "list", "data": serde_json::Value::Null });
    let models = parse_models_response(&data_null)
        .expect("null `data` must parse as an empty catalog, not an error");
    assert!(
        models.is_empty(),
        "null `data` must yield an empty model list, got {models:?}"
    );

    // The sibling `models` key (Codex-shaped envelope) gets the same treatment.
    let models_null = serde_json::json!({ "object": "list", "models": serde_json::Value::Null });
    let models = parse_models_response(&models_null)
        .expect("null `models` must parse as an empty catalog, not an error");
    assert!(
        models.is_empty(),
        "null `models` must yield an empty model list, got {models:?}"
    );

    // A bare success envelope with no `object` field still null-encodes an
    // empty catalog (treated as success — `object` absent ⇒ not an error).
    let object_absent = serde_json::json!({ "data": serde_json::Value::Null });
    let models = parse_models_response(&object_absent)
        .expect("null `data` with no `object` field must parse as an empty catalog");
    assert!(
        models.is_empty(),
        "null `data` (object absent) must yield an empty model list, got {models:?}"
    );
}

#[test]
fn parse_models_response_rejects_null_data_on_error_envelope() {
    // Codex P2 (PR #4157): an HTTP-200 error body such as
    // `{"object":"error","data":null}` ALSO null-encodes `data`. The
    // null-as-empty short-circuit MUST NOT swallow it as a successful empty
    // catalog — that would hide provider/endpoint failures from the UI and
    // Sentry. A non-"list" `object` with null `data` falls through to the
    // descriptive malformed/error-envelope error, which surfaces `object`.
    for field in ["data", "models"] {
        let body = serde_json::json!({ "object": "error", field: serde_json::Value::Null });
        let err = match parse_models_response(&body) {
            Ok(models) => panic!(
                "null `{field}` on an error envelope must fail, not return empty (got {models:?})"
            ),
            Err(err) => err,
        };
        assert!(
            !err.contains("missing"),
            "error-envelope null `{field}` must not say `missing`: {err}"
        );
        // Tighten on the surfaced `object` value, not the literal "error
        // envelope" prose, so the assertion proves the provider error is
        // actually carried through to triage.
        assert!(
            err.contains(r#""object" = "error""#),
            "error-envelope null `{field}` must surface `\"object\" = \"error\"`: {err}"
        );
    }
}

#[test]
fn openai_codex_model_hints_are_merged_without_duplicates() {
    let mut models = vec![ModelInfo {
        id: "gpt-5.4".to_string(),
        owned_by: Some("openai-codex".to_string()),
        context_window: Some(128000),
        display_name: None,
        input_per_1m: None,
        output_per_1m: None,
    }];

    merge_openai_codex_model_hints(&mut models);

    let ids = models
        .iter()
        .map(|model| model.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec!["gpt-5.4", "gpt-5.5", "gpt-5.3-codex-spark", "gpt-5.3-codex"]
    );
}

// ── synthesize_local_runtime_entry (TAURI-RUST-28Z fallback) ────────────

#[test]
fn synthesize_local_runtime_entry_ollama_returns_v1_endpoint_with_no_auth() {
    // Sentry TAURI-RUST-28Z fires when `inference_list_models("ollama")`
    // runs against a config that has no `ollama` cloud_providers entry.
    // The synth fallback must produce an entry routed to Ollama's
    // OpenAI-compatible `/v1/models` surface at the resolved base URL,
    // with `AuthStyle::None` so the probe runs without an Authorization
    // header (loopback Ollama accepts unauthenticated requests).
    let config = Config::default();
    let entry = synthesize_local_runtime_entry("ollama", &config)
        .expect("ollama must produce a synthetic entry");
    assert_eq!(entry.slug, "ollama");
    assert_eq!(entry.auth_style, AuthStyle::None);
    assert!(
        entry.endpoint.ends_with("/v1"),
        "ollama endpoint must terminate at /v1 so `<endpoint>/models` hits the OpenAI-compat surface; got {}",
        entry.endpoint
    );
}

#[test]
fn synthesize_local_runtime_entry_lmstudio_returns_v1_endpoint_with_no_auth() {
    // LM Studio's default `lm_studio_base_url` already terminates at
    // `/v1`; the synth must preserve that and select `AuthStyle::None`
    // so the probe doesn't attach a bearer header (LM Studio runs
    // unauthenticated on loopback).
    let config = Config::default();
    let entry = synthesize_local_runtime_entry("lmstudio", &config)
        .expect("lmstudio must produce a synthetic entry");
    assert_eq!(entry.slug, "lmstudio");
    assert_eq!(entry.auth_style, AuthStyle::None);
    assert!(
        entry.endpoint.ends_with("/v1"),
        "lmstudio endpoint must terminate at /v1; got {}",
        entry.endpoint
    );
}

#[test]
fn synthesize_local_runtime_entry_returns_none_for_unknown_slug() {
    // Only `ollama` and `lmstudio` are the recognized local-runtime
    // aliases. Every other slug — built-in cloud providers (`openai`,
    // `anthropic`), opaque ids (`p_random_xyz`), or typos — must fall
    // through to the existing "no cloud provider" error. Pinning this
    // rejection contract guards against the synth growing into a
    // blanket "any unknown slug points at localhost" matcher.
    let config = Config::default();
    for slug in ["openai", "anthropic", "openrouter", "p_random_xyz", "", " "] {
        assert!(
            synthesize_local_runtime_entry(slug, &config).is_none(),
            "{slug:?} must NOT synthesize a local-runtime entry"
        );
    }
}

#[test]
fn parse_models_response_handles_non_object_body() {
    // Provider returned a bare array / string / number at the
    // top level — not an object at all. Surface as a parse failure
    // (not a panic).
    for body in [
        serde_json::json!([{"id": "m1"}]),
        serde_json::json!("hello"),
        serde_json::Value::Null,
    ] {
        let err = parse_models_response(&body)
            .expect_err("non-object body must fail with a clear message");
        assert!(
            !err.is_empty(),
            "non-object body error must be non-empty: {err}"
        );
    }
}

/// `is_backend_auth_failure` is the polarity guard that decides whether a
/// 401/403 is the OpenHuman backend's expired session (silence + drive
/// reauth) or a third-party BYO-key rejection (actionable, must reach
/// Sentry). Getting this wrong in either direction is a regression:
/// over-matching silences real misconfig; under-matching is TAURI-RUST-N.
#[test]
fn is_backend_auth_failure_only_matches_openhuman_backend_401_403() {
    use reqwest::StatusCode;
    let backend = crate::openhuman::inference::provider::openhuman_backend_model::PROVIDER_LABEL;

    assert!(is_backend_auth_failure(backend, StatusCode::UNAUTHORIZED));
    assert!(is_backend_auth_failure(backend, StatusCode::FORBIDDEN));

    // Non-auth backend statuses stay reportable (real server bugs / transient).
    for s in [
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::BAD_REQUEST,
        StatusCode::NOT_FOUND,
    ] {
        assert!(
            !is_backend_auth_failure(backend, s),
            "backend {s} must not be treated as session-expiry"
        );
    }

    // Third-party BYO-key 401/403 (user's own key revoked) must NOT be
    // silenced — that is actionable misconfiguration for Sentry.
    for provider in ["custom_openai", "OpenAI", "Anthropic", "openrouter"] {
        assert!(
            !is_backend_auth_failure(provider, StatusCode::UNAUTHORIZED),
            "{provider} 401 must reach Sentry as actionable BYO-key error"
        );
        assert!(
            !is_backend_auth_failure(provider, StatusCode::FORBIDDEN),
            "{provider} 403 must reach Sentry as actionable BYO-key error"
        );
    }
}

/// `is_byo_provider_auth_failure_http` demotes a non-backend provider's
/// 401/403 from Sentry when the body looks like a missing/invalid BYO API
/// key (TAURI-RUST-DHM: a `kiro` custom provider with no key flooded Sentry
/// with 5,636 identical events from one user via the memory-tree retry loop).
/// The gate is provider-scoped (backend keeps its SessionExpired branch) and
/// body-shape-anchored (a non-auth 401, e.g. quota / geo-block, still reports).
#[test]
fn byo_provider_auth_failure_demotes_authentication_error_bodies() {
    use reqwest::StatusCode;

    // The exact kiro 401 envelope from the Sentry report.
    let kiro_body =
        r#"{"error":{"message":"Invalid or missing API key","type":"authentication_error"}}"#;
    assert!(is_byo_provider_auth_failure_http(
        "kiro",
        StatusCode::UNAUTHORIZED,
        kiro_body
    ));
    // 403 with the same envelope is demoted too.
    assert!(is_byo_provider_auth_failure_http(
        "kiro",
        StatusCode::FORBIDDEN,
        kiro_body
    ));

    // Every recognised auth-key marker across the BYO providers in Sentry.
    for body in [
        r#"{"error":{"type":"authentication_error"}}"#,
        r#"{"error":{"code":"invalid_api_key","message":"Incorrect API key provided"}}"#,
        "Invalid API key",
        "invalid or missing api key",
        "missing api key",
        r#"{"message":"no api key supplied"}"#,
        "invalid authentication",
    ] {
        assert!(
            is_byo_provider_auth_failure_http("custom_openai", StatusCode::UNAUTHORIZED, body),
            "BYO auth body must be demoted: {body}"
        );
    }
}

/// The backend keeps its `is_backend_auth_failure` → SessionExpired branch:
/// a backend 401 with an auth-error body must NOT be swallowed here, or the
/// session-expiry reauth path (and its existing test) would silently break.
#[test]
fn byo_provider_auth_failure_excludes_openhuman_backend() {
    use reqwest::StatusCode;
    let backend = crate::openhuman::inference::provider::openhuman_backend_model::PROVIDER_LABEL;
    let body = r#"{"error":{"type":"authentication_error"}}"#;
    assert!(!is_byo_provider_auth_failure_http(
        backend,
        StatusCode::UNAUTHORIZED,
        body
    ));
    assert!(!is_byo_provider_auth_failure_http(
        backend,
        StatusCode::FORBIDDEN,
        body
    ));
}

/// Body-shape anchoring: a 401/403 whose body is NOT an auth-key envelope
/// (quota, geo-block, opaque gateway text) still reaches Sentry — the gate
/// keys on the body, not the bare status. And a non-401/403 status with an
/// auth-ish body is out of scope (handled by the budget / config-rejection
/// branches or the status gate).
#[test]
fn byo_provider_auth_failure_is_body_and_status_scoped() {
    use reqwest::StatusCode;

    // 401/403 without an auth-key envelope — still actionable, must report.
    for body in [
        r#"{"error":{"message":"Access denied for your region","type":"forbidden"}}"#,
        r#"{"error":{"message":"Quota exceeded for this account"}}"#,
        "Unauthorized",
    ] {
        assert!(
            !is_byo_provider_auth_failure_http("custom_openai", StatusCode::UNAUTHORIZED, body),
            "non-auth-envelope body must stay reportable: {body}"
        );
    }

    // Auth-shaped body on a non-401/403 status is out of this predicate's scope.
    for status in [
        StatusCode::BAD_REQUEST,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::NOT_FOUND,
    ] {
        assert!(
            !is_byo_provider_auth_failure_http(
                "custom_openai",
                status,
                r#"{"error":{"type":"authentication_error"}}"#
            ),
            "status {status} with auth body must not be demoted here"
        );
    }
}

/// End-to-end through `api_error`: a non-backend 401 with an auth-error body
/// returns the sanitized provider error (so the chat/UI surface is unchanged)
/// while the BYO-auth branch demotes it from Sentry. Exercises the wired-in
/// cascade, not just the predicate in isolation.
#[tokio::test]
async fn api_error_byo_auth_failure_returns_message_via_demoted_branch() {
    let http_response = axum::http::Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .body(
            r#"{"error":{"message":"Invalid or missing API key","type":"authentication_error"}}"#
                .to_string(),
        )
        .expect("build 401 response");
    let response = reqwest::Response::from(http_response);

    let err = api_error("kiro", response).await;
    let msg = err.to_string();
    assert!(
        msg.contains("kiro API error (401"),
        "error must still carry the provider/status prefix for the UI: {msg}"
    );
    assert!(
        msg.to_ascii_lowercase()
            .contains("invalid or missing api key"),
        "sanitized upstream body must propagate to the caller: {msg}"
    );
}

/// End-to-end through `api_error`: a 500-wrapped monthly-quota refusal (the
/// Kiro IDE proxy nests its 402 / `MONTHLY_REQUEST_COUNT` inside a 500
/// envelope, TAURI-RUST-C9A) returns the sanitized provider error to the UI
/// while routing through the quota-exhausted demote branch — *before* the
/// `should_report_provider_http_failure(500)` status gate that would otherwise
/// page once per memory-extraction retry.
#[tokio::test]
async fn api_error_monthly_quota_returns_message_via_demoted_branch() {
    let body = "{\"error\":{\"message\":\"HTTP 402 from Kiro IDE: \
        {\\\"message\\\":\\\"You have reached the limit.\\\",\
        \\\"reason\\\":\\\"MONTHLY_REQUEST_COUNT\\\"}\",\"type\":\"server_error\"}}";
    let http_response = axum::http::Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .body(body.to_string())
        .expect("build 500 response");
    let response = reqwest::Response::from(http_response);

    let err = api_error("kiro", response).await;
    let msg = err.to_string();
    assert!(
        msg.contains("kiro API error (500"),
        "error must still carry the provider/status prefix for the UI: {msg}"
    );
    assert!(
        msg.contains("MONTHLY_REQUEST_COUNT"),
        "sanitized upstream quota body must propagate to the caller: {msg}"
    );
    // The body must classify as quota-exhausted so the demote branch — not the
    // 500 status gate — handles it.
    assert!(is_provider_quota_exhausted(body));
    assert!(should_report_provider_http_failure(
        StatusCode::INTERNAL_SERVER_ERROR
    ));
}

/// `publish_backend_session_expired` must emit a `SessionExpired` event on
/// the `auth` domain with the canonical source and a sanitized reason, so
/// the credentials subscriber can drive reauth.
#[tokio::test]
async fn publish_backend_session_expired_emits_sanitized_session_expired() {
    use crate::core::events::DomainEvent;

    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS
        .get()
        .expect("event bus initialized")
        .receiver();

    // `TEST_MARKER_A` makes this event distinguishable from the sibling
    // `chat_completions_backend_401_*` test's event on the shared global
    // bus (both run in parallel against the same singleton). The `sk-`
    // token probes that `sanitize_api_error` actually scrubs secrets out
    // of the SessionExpired reason rather than just emitting the event.
    let secret = "sk-LIVEA0123456789abcdefSECRET";
    let msg = format!(
        r#"OpenHuman API error (401 Unauthorized): {{"success":false,"error":"TEST_MARKER_A Invalid token {secret}"}}"#
    );
    publish_backend_session_expired(
        "chat_completions",
        crate::openhuman::inference::provider::openhuman_backend_model::PROVIDER_LABEL,
        reqwest::StatusCode::UNAUTHORIZED,
        &msg,
    );

    let mut reason_seen: Option<String> = None;
    loop {
        match rx.try_recv() {
            Ok(DomainEvent::SessionExpired { source, reason }) => {
                if source == "llm_provider.openhuman_backend" && reason.contains("TEST_MARKER_A") {
                    reason_seen = Some(reason);
                    break;
                }
            }
            Ok(_) => continue,
            Err(tinybus::TryRecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
    let reason = reason_seen.expect(
        "publish_backend_session_expired must emit SessionExpired(source=llm_provider.openhuman_backend) carrying TEST_MARKER_A",
    );
    assert!(
        reason.contains("[REDACTED]"),
        "sanitize_api_error must redact the sk- token in the reason: {reason}"
    );
    assert!(
        !reason.contains(secret),
        "raw secret must not survive into the SessionExpired reason: {reason}"
    );
}

#[test]
fn synthesize_local_runtime_entry_ollama_respects_config_base_url() {
    // The synth must honor `config.local_ai.base_url` (the same
    // priority `ollama_base_url_from_config` uses for chat routing).
    // This is the path users hit when they point Ollama at a non-loopback
    // host (e.g. a LAN box at 192.168.1.5).
    let mut config = Config::default();
    config.local_ai.base_url = Some("http://192.168.1.5:11434".to_string());
    let entry = synthesize_local_runtime_entry("ollama", &config)
        .expect("ollama with custom base_url must still synthesize");
    assert_eq!(
        entry.endpoint, "http://192.168.1.5:11434/v1",
        "synth must use config.local_ai.base_url and append /v1 once",
    );
}

#[test]
fn cloud_providers_entry_takes_precedence_over_local_runtime_synthesis() {
    // Pin the precedence: if the user has explicitly added an `ollama`
    // entry to `cloud_providers` (e.g. a remote ollama box at
    // https://ollama.example.com/v1), that entry MUST win — the synth
    // fallback is reached only when the find returns `None`. Mirrors
    // the lookup in `list_configured_models_from_config` so a future
    // refactor that swaps `find().or_else(synth)` for unconditional
    // synthesis fails this test loudly.
    let mut config = Config::default();
    config.cloud_providers.push(CloudProviderCreds {
        id: "p_ollama_explicit".to_string(),
        slug: "ollama".to_string(),
        label: "Remote Ollama".to_string(),
        endpoint: "https://ollama.example.com/v1".to_string(),
        auth_style: AuthStyle::Bearer,
        legacy_type: None,
        default_model: None,
    });

    let resolved = config
        .cloud_providers
        .iter()
        .find(|e| e.id == "ollama" || e.slug == "ollama")
        .cloned()
        .or_else(|| synthesize_local_runtime_entry("ollama", &config))
        .expect("either explicit or synth must resolve");
    assert_eq!(
        resolved.endpoint, "https://ollama.example.com/v1",
        "explicit cloud_providers entry must beat local-runtime synth",
    );
    assert_eq!(resolved.auth_style, AuthStyle::Bearer);
}

#[test]
fn missing_cloud_providers_entry_falls_back_to_local_runtime_synth() {
    // The TAURI-RUST-28Z regression contract: when no `ollama` entry
    // exists in `cloud_providers` AND the slug is a recognized
    // local-runtime alias, the find/synth chain must yield a synthetic
    // entry (instead of `None`, which produces the
    // "no cloud provider with id or slug 'ollama' found" Sentry error).
    let config = Config::default();
    assert!(
        config.cloud_providers.is_empty(),
        "precondition: clean config has no providers configured",
    );

    let resolved = config
        .cloud_providers
        .iter()
        .find(|e| e.id == "ollama" || e.slug == "ollama")
        .cloned()
        .or_else(|| synthesize_local_runtime_entry("ollama", &config));
    assert!(
        resolved.is_some(),
        "ollama must resolve via synth when cloud_providers is empty"
    );
    assert_eq!(resolved.unwrap().slug, "ollama");
}

#[test]
fn missing_cloud_providers_entry_for_unknown_slug_still_errors() {
    // The synth is intentionally narrow: only `ollama` and `lmstudio`
    // get fallback routing. An unknown slug with no `cloud_providers`
    // match must continue to produce `None` (which the caller surfaces
    // as the "no cloud provider" error) — otherwise typos would
    // silently route to localhost.
    let config = Config::default();
    let resolved = config
        .cloud_providers
        .iter()
        .find(|e| e.id == "tpyo" || e.slug == "tpyo")
        .cloned()
        .or_else(|| synthesize_local_runtime_entry("tpyo", &config));
    assert!(
        resolved.is_none(),
        "unknown slug with no cloud_providers entry must NOT synthesize",
    );
}
