use super::*;

#[test]
fn does_not_classify_generic_breaker_messages() {
    // A generic "circuit breaker open" must not be silenced: other domains
    // may use the same phrase for real bugs that need to reach Sentry.
    assert_eq!(
        expected_error_kind("provider reliability: circuit breaker open for openai"),
        None
    );
}

#[test]
fn does_not_classify_unrelated_messages_as_capability_unavailable() {
    // The classifier anchors on the exact "for this RAM tier" substring.
    // Messages that talk about RAM in a different context (sizing the
    // tier list, doc references) must not be silenced.
    assert_eq!(expected_error_kind("ollama embed failed: out of RAM"), None);
    assert_eq!(
        expected_error_kind("local_ai_set_ram_tier failed: invalid tier value"),
        None
    );
}

#[test]
fn classifies_network_unreachable_errors() {
    // OPENHUMAN-TAURI-32: reqwest's transport-level error wrapped by the
    // web_channel error site. The classifier must catch it even when
    // embedded in caller context, since `report_error_or_expected` runs
    // `expected_error_kind` on the full anyhow chain.
    assert_eq!(
        expected_error_kind(
            "run_chat_task failed client_id=abc thread_id=t1 request_id=r1 \
             error=error sending request for url (https://api.tinyhumans.ai/openai/v1/chat/completions)"
        ),
        Some(ExpectedErrorKind::NetworkUnreachable)
    );
    for raw in [
        "error sending request for url (https://api.example.com/x)",
        "provider failed: dns error: failed to lookup address information",
        "tcp connect: connection refused (os error 61)",
        "stream closed: connection reset by peer",
        "network is unreachable (os error 51)",
        "no route to host",
        "tls handshake eof",
        "certificate verify failed: unable to get local issuer certificate",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::NetworkUnreachable),
            "should classify as network-unreachable: {raw}"
        );
    }
}

#[test]
fn custom_openai_ollama_timeout_fallback_chain_is_network_unreachable() {
    let chain = "custom_openai chat completions transport error: error sending request for url \
                 (http://localhost:11434/v1/chat/completions): operation timed out \
                 (responses fallback failed: custom_openai API error (404 Not Found): \
                 {\"error\":\"not found\"})";
    assert_eq!(
        expected_error_kind(chain),
        Some(ExpectedErrorKind::NetworkUnreachable),
        "local Ollama timeout plus responses fallback failure must not page Sentry"
    );
}

#[test]
fn does_not_classify_unrelated_provider_errors_as_network() {
    // Status-bearing provider failures (404, 500, …) are surfaced via
    // their HTTP status path and must NOT be silenced by the
    // network-unreachable classifier — the body text doesn't hit any of
    // the transport-level markers.
    assert_eq!(
        expected_error_kind("OpenAI API error (404): model gpt-x not found"),
        None
    );
    assert_eq!(
        expected_error_kind("OpenAI API error (500): internal server error"),
        None
    );
}

#[test]
fn classifies_wave4_socket_transport_wire_shapes() {
    // OPENHUMAN-TAURI-44 (~50 events): libc `getaddrinfo()` rendering
    // without the `dns error` token, wrapped by the socket emit site.
    // The Wave 4 matcher arms catch the literal resolver phrases that
    // the original `dns error` substring would miss when reqwest's
    // wrapper isn't in the chain (e.g. tungstenite IO errors).
    assert_eq!(
        expected_error_kind(
            "[socket] Connection failed (sustained outage after 5 attempts): \
             WebSocket connect: IO error: failed to lookup address information: \
             nodename nor servname provided, or not known"
        ),
        Some(ExpectedErrorKind::NetworkUnreachable)
    );

    // OPENHUMAN-TAURI-4P (~66 events): tungstenite renders a captive
    // portal / corporate proxy that intercepts the WS handshake as
    // `WsError::Http(200)` → `"HTTP error: 200 OK"`. Classify as
    // network-unreachable since no amount of app-side retry can pierce
    // an intercepting proxy.
    assert_eq!(
        expected_error_kind(
            "[socket] Connection failed (sustained outage after 5 attempts): \
             WebSocket connect: HTTP error: 200 OK"
        ),
        Some(ExpectedErrorKind::NetworkUnreachable)
    );
}

#[test]
fn classifies_socket_connect_deadline_as_network_unreachable() {
    // #6256: `connect_with_redirects_within` renders its per-hop deadline as
    // an `io::ErrorKind::TimedOut` carrying the `operation timed out` phrase,
    // wrapped by `run_connection` and the supervisor's sustained-outage
    // escalation like every other connect-stage failure. A path that keeps
    // blackholing the upgrade is user-environment / edge state the reconnect
    // loop already retries — it must demote, never page.
    assert_eq!(
        expected_error_kind(
            "[socket] Connection failed (sustained outage after 5 attempts): \
             WebSocket connect: IO error: operation timed out after 10s waiting \
             for the WebSocket upgrade"
        ),
        Some(ExpectedErrorKind::NetworkUnreachable)
    );
}

#[test]
fn http_200_classifier_does_not_silence_unrelated_log_lines() {
    // The captive-portal arm anchors on `"http error: 200 ok"` (the
    // exact tungstenite `WsError::Http(200)` Display rendering).
    // Adjacent non-WebSocket log lines that mention `"HTTP/1.1 200 OK"`
    // or `"status: 200 OK"` MUST NOT classify — those are normal-flow
    // success traces, not failure events. Pin this precedence so a
    // future refactor doesn't broaden the substring.
    assert_eq!(expected_error_kind("HTTP/1.1 200 OK"), None);
    assert_eq!(
        expected_error_kind("upstream returned status: 200 OK after retry"),
        None
    );
}

#[test]
fn classifies_tls_handshake_eof_as_network_unreachable() {
    // TAURI-RUST-4ZD (first seen on `openhuman@0.56.0+e8968077aeb5`,
    // Windows): `native-tls` renders a peer / firewall / antivirus /
    // corporate-proxy TCP close mid-TLS-handshake as
    // `"TLS error: native-tls error: unexpected EOF during handshake"`,
    // which `socket::ws_loop::run_connection` wraps as
    // `"WebSocket connect: <inner>"` and the supervisor's
    // sustained-outage escalation wraps again. The existing
    // `"tls handshake"` arm misses it because the words are not
    // contiguous in this render (`"tls error"` … `"during handshake"`).
    // Same user-environment shape as the other handshake-stage entries:
    // the socket supervisor already retries with exponential backoff and
    // Sentry has no actionable signal beyond that.
    assert_eq!(
        expected_error_kind(
            "[socket] Connection failed (sustained outage after 5 attempts): \
             WebSocket connect: TLS error: native-tls error: unexpected EOF during handshake"
        ),
        Some(ExpectedErrorKind::NetworkUnreachable)
    );

    // Bare native-tls render (no socket-supervisor wrap) — fires when the
    // same handshake EOF escapes through a non-supervisor call site. The
    // classifier runs on the full anyhow chain, so the shorter form must
    // also match.
    assert_eq!(
        expected_error_kind("TLS error: native-tls error: unexpected EOF during handshake"),
        Some(ExpectedErrorKind::NetworkUnreachable)
    );
}

#[test]
fn classifies_ws_protocol_wrong_http_version_as_network_unreachable() {
    // CORE-RUST-DP (~2 events / 24h on `openhuman@0.56.0+e8968077aeb5`,
    // self-hosted `core-rust`): tungstenite renders
    // `ProtocolError::WrongHttpVersion` as
    // `"WebSocket protocol error: HTTP version must be 1.1 or higher"`,
    // wrapped by `socket::ws_loop::run_connection` as
    // `"WebSocket connect: <inner>"` and then by the supervisor's
    // sustained-outage escalation as
    // `"[socket] Connection failed (sustained outage after N attempts):
    // WebSocket connect: WebSocket protocol error: HTTP version must be
    // 1.1 or higher"`.
    //
    // The handshake requires HTTP/1.1; a server or intermediary proxy
    // that responds with HTTP/2+ to the upgrade is misconfigured
    // upstream — same shape as the existing `"tls handshake"` /
    // `"certificate verify failed"` user-environment entries. The
    // supervisor already retries with exponential backoff; Sentry has
    // no actionable signal to add.
    assert_eq!(
        expected_error_kind(
            "[socket] Connection failed (sustained outage after 5 attempts): \
             WebSocket connect: WebSocket protocol error: HTTP version must be 1.1 or higher"
        ),
        Some(ExpectedErrorKind::NetworkUnreachable)
    );

    // Bare tungstenite render (no socket-supervisor wrap) — fires when
    // the same protocol error escapes through a non-supervisor call
    // site. The classifier runs on the full anyhow chain, so the
    // shorter form must also match.
    assert_eq!(
        expected_error_kind("WebSocket protocol error: HTTP version must be 1.1 or higher"),
        Some(ExpectedErrorKind::NetworkUnreachable)
    );
}

#[test]
fn tls_handshake_eof_anchor_does_not_silence_unrelated_log_lines() {
    // The anchor is the literal `"unexpected eof during handshake"`
    // phrase. A bare data-phase `"unexpected EOF"` (server closed
    // mid-stream, parser truncation, …) MUST NOT classify — those are
    // outside the handshake stage and may carry actionable signal. Pin
    // the rejection contract so a future refactor doesn't loosen the
    // substring into a generic `"unexpected eof"` matcher.
    for raw in [
        "stream closed: unexpected EOF",
        "reqwest: unexpected EOF while reading body",
        "json parser: unexpected EOF at byte 1024",
        "decoder hit unexpected eof mid-frame",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            None,
            "non-handshake unexpected-EOF log line must NOT classify: {raw}"
        );
    }
}

#[test]
fn wrong_http_version_anchor_does_not_silence_unrelated_log_lines() {
    // The anchor is the literal tungstenite Display string. Adjacent
    // log lines that mention HTTP version in any other context
    // (`"upgrading from HTTP/1.0 to HTTP/2"`, `"HTTP/1.1 only"`,
    // `"server requires HTTP version 2.0"`) MUST NOT classify — those
    // are unrelated transport / negotiation traces and may carry
    // actionable signal. Pin the rejection contract so a future
    // refactor doesn't loosen the substring into a generic
    // `"http version"` matcher.
    for raw in [
        "[transport] upgrading from HTTP/1.0 to HTTP/2",
        "server advertises HTTP version 2.0 (h2 alpn)",
        "client supports HTTP/1.1 only",
        "version mismatch: requires HTTP/1.2 or higher",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            None,
            "unrelated HTTP-version log line must NOT classify: {raw}"
        );
    }
}

#[test]
fn classifies_transient_upstream_http_errors() {
    // OPENHUMAN-TAURI-5Z: the canonical shape emitted by
    // `providers::ops::api_error` and re-raised through `agent.run_single`.
    assert_eq!(
        expected_error_kind("OpenHuman API error (504 Gateway Timeout): error code: 504"),
        Some(ExpectedErrorKind::TransientUpstreamHttp)
    );

    // Every transient code must classify, whether the status renders as
    // bare digits or "<digits> <reason>".
    for raw in [
        "OpenHuman API error (408): request timeout",
        "OpenAI API error (429 Too Many Requests): rate limit",
        "Anthropic API error (502 Bad Gateway): upstream unhealthy",
        "custom_openai API error (502 Bad Gateway): upstream gateway blip",
        "OpenHuman API error (503): service unavailable",
        "Provider API error (504): upstream timed out",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::TransientUpstreamHttp),
            "should classify as transient upstream HTTP: {raw}"
        );
    }

    // Wrapped in an anyhow chain (as it reaches the agent layer) must
    // still classify — `expected_error_kind` is substring-based.
    assert_eq!(
        expected_error_kind(
            "agent turn failed: OpenHuman API error (504 Gateway Timeout): \
             error code: 504"
        ),
        Some(ExpectedErrorKind::TransientUpstreamHttp)
    );

    // TAURI-RUST-H (~1360 events, 504) / TAURI-RUST-2T (~310 events, 502):
    // legacy no-paren wire shape from older `embeddings::openai` /
    // `embeddings::cohere` emit-site formats that predate the
    // parenthesised `({status})` rendering. Anchored on the trailing
    // space after the status code so unrelated digit runs don't match.
    for raw in [
        "Embedding API error 504 Gateway Timeout: error code: 504",
        "Embedding API error 502 Bad Gateway: error code: 502",
        "Cohere embed API error 503 Service Unavailable: error code: 503",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::TransientUpstreamHttp),
            "should classify legacy no-paren transient shape: {raw}"
        );
    }
}

#[test]
fn does_not_classify_unrelated_digit_runs_as_transient() {
    // The legacy no-paren matcher anchors `api error <code> ` with a
    // trailing space so adjacent digit runs (`api error 5042…`) and
    // non-transient codes (400/401/403/404) don't get silenced.
    assert_eq!(
        expected_error_kind("OpenHuman API error 400 Bad Request: malformed body"),
        None
    );
    assert_eq!(
        expected_error_kind("provider returned api error 5042 (custom internal sentinel)"),
        None
    );
}

#[test]
fn integrations_post_composio_timeout_dropped() {
    // OPENHUMAN-TAURI-18 / -G regression guard. The integrations
    // client at `crate::integrations::client::IntegrationClient::post`
    // builds the reqwest error chain and routes it through
    // `report_error_or_expected(.., "integrations", "post", &[("failure",
    // "transport")])`. The chain text contains the
    // `"error sending request for url"` anchor so
    // `is_network_unreachable_message` matches first and demotes to
    // `NetworkUnreachable` (functionally equivalent to
    // `TransientUpstreamHttp` for Sentry suppression — both routes
    // skip the report path via `report_expected_message`).
    //
    // Pinning this exact wire shape catches a future refactor that
    // drops the URL anchor (e.g. a chain-flatten helper that strips
    // it for "PII safety"), which would silently re-open the leak.
    let chain = "error sending request for url \
                 (https://api.tinyhumans.ai/agent-integrations/composio/execute) → \
                 client error (SendRequest) → connection error → \
                 Operation timed out (os error 60)";
    assert_eq!(
        expected_error_kind(chain),
        Some(ExpectedErrorKind::NetworkUnreachable),
        "TAURI-18 chain shape must classify as NetworkUnreachable"
    );

    // If the URL anchor is ever dropped, the transport-phrase
    // fallback (`operation timed out` from
    // `TRANSIENT_TRANSPORT_PHRASES`) catches it via the message
    // classifier helper used at upstream re-emit sites — confirm
    // both paths so the regression surface is fully pinned.
    assert!(
        is_transient_message_failure(chain),
        "TAURI-18 chain must also satisfy upstream message classifier \
         (defense-in-depth for sites that lose the URL anchor)"
    );
}

#[test]
fn channel_supervisor_operation_timed_out_classifies_as_expected() {
    // OPENHUMAN-TAURI-EM (128 events) + TAURI-RUST-15/-BB: `channels::runtime::supervision`
    // wraps a channel listener failure as
    // `format!("Channel {} error: {e:#}; restarting", ch.name())` and
    // routes the message through `report_error_or_expected`. The
    // newer `ChannelSupervisorRestart` classifier (added for the
    // broader 11.4k-event Sentry leak) anchors on the supervisor
    // wrapper shape itself — `"Channel <name> error: …; restarting"`
    // — and takes precedence over `NetworkUnreachable`. That single
    // arm now covers every ETIMEDOUT / WSAETIMEDOUT / hyper-prose
    // shape the old narrower anchor pinned, plus OS-localized
    // variants the English-only `NetworkUnreachable` would miss.
    //
    // Demotion tier difference: `ChannelSupervisorRestart` emits at
    // `info!` (breadcrumb only, no Sentry event) where
    // `NetworkUnreachable` emitted at `warn!` (still captured as a
    // Sentry warn event). Sustained outages still page via
    // `health.bus` / `FAIL_ESCALATE_THRESHOLD`.
    for raw in [
        // macOS (os error 60 = ETIMEDOUT on BSD)
        "Channel discord error: IO error: Operation timed out (os error 60); restarting",
        // Linux (os error 110 = ETIMEDOUT)
        "Channel discord error: IO error: Operation timed out (os error 110); restarting",
        // Windows (os error 10060 = WSAETIMEDOUT)
        "Channel discord error: IO error: Operation timed out (os error 10060); restarting",
        // Same shape on other channels — supervisor wrapper is provider-agnostic.
        "Channel slack error: IO error: Operation timed out (os error 60); restarting",
        "Channel telegram error: IO error: Operation timed out (os error 110); restarting",
        // Bare prose form (no errno suffix) from hyper / tungstenite layers
        // that render `std::io::Error` without `raw_os_error()`.
        "Channel discord error: WebSocket connect: IO error: Operation timed out; restarting",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::ChannelSupervisorRestart),
            "channel supervisor timeout shape must classify as ChannelSupervisorRestart \
             (precedence over NetworkUnreachable; got {:?} for {raw:?})",
            expected_error_kind(raw)
        );
    }
}

#[test]
fn operation_timed_out_negative_cases_still_report() {
    // Counter-case: a configuration/validation message that mentions
    // "timeout" as a knob name (not transport state) and has no other
    // classifier anchor must still reach Sentry. The substring chosen
    // for the new matcher is `"operation timed out"`, not `"timeout"`,
    // precisely so unrelated mentions of the word do not collide.
    assert_eq!(
        expected_error_kind("config rejected: timeout must be a positive integer"),
        None,
        "config validation noise (no 'operation timed out' anchor) must still reach Sentry"
    );
    // Bare empty string — no anchors at all.
    assert_eq!(expected_error_kind(""), None);
}

#[test]
fn channels_dispatch_re_emit_of_provider_502_classifies_as_transient() {
    // OPENHUMAN-TAURI-4F (~157 events) / -1C (~87 events) / -8F
    // (~39 events): the reliable provider layer retried 5xx, the
    // agent re-raised the error, and `channels::runtime::dispatch`
    // re-emitted it under `domain="channels", operation="dispatch_llm_error"`
    // via raw `report_error` (which skips classification). Switching
    // that site to `report_error_or_expected` routes the chain
    // through this classifier — but only works if the canonical
    // `"OpenHuman API error (NNN ...)"` substring still anchors the
    // match through the channels-layer wrapping.
    //
    // The wrapping shape at the dispatch site is the agent error
    // chain rendered via `format!("{e:#}")`. For a backend 502 from
    // `providers::ops::api_error`, that resolves to:
    //   "OpenHuman API error (502 Bad Gateway): error code: 502"
    // possibly prepended with a runner / iteration prefix. Both
    // shapes must classify as transient so the dispatch re-emit
    // gets demoted.
    for raw in [
        "OpenHuman API error (502 Bad Gateway): error code: 502",
        "agent.provider_chat failed: OpenHuman API error (503 Service Unavailable): retry budget exhausted",
        "all providers exhausted: OpenHuman API error (504 Gateway Timeout): error code: 504",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::TransientUpstreamHttp),
            "channels.dispatch re-emit of {raw:?} must classify as transient"
        );
    }
}

#[test]
fn classifies_socket_transient_http_errors() {
    // OPENHUMAN-TAURI-5P / -EZ: tungstenite's `WsError::Http(response)`
    // surfaces during the WebSocket upgrade handshake when the backend
    // load balancer returns 502 / 504. The socket reconnect loop wraps
    // it as `format!("WebSocket connect: {e}")`, producing
    // `"WebSocket connect: HTTP error: <status> <reason>"`. Each
    // sustained-outage threshold escalation routes the formatted reason
    // through `report_error_or_expected`, which must classify as
    // transient so the per-client noise stops reaching Sentry.
    for raw in [
        "WebSocket connect: HTTP error: 502 Bad Gateway",
        "WebSocket connect: HTTP error: 503 Service Unavailable",
        "WebSocket connect: HTTP error: 504 Gateway Timeout",
        "[socket] Connection failed (sustained outage after 5 attempts): \
         WebSocket connect: HTTP error: 502 Bad Gateway",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::TransientUpstreamHttp),
            "should classify as transient upstream HTTP (socket shape): {raw}"
        );
    }

    // Trailing-colon separator (chained error formatting).
    // Note: avoid words like "connection refused" or "timeout" in the
    // suffix — those would also match `is_network_unreachable_message` /
    // `TRANSIENT_TRANSPORT_PHRASES` and the order in `expected_error_kind`
    // would route through `NetworkUnreachable` first, defeating the
    // assertion. Both classifications silence the event so production
    // behavior is identical, but the test is anchored on the canonical
    // socket shape so a future regression in `is_transient_upstream_http_message`
    // surfaces here, not behind another classifier.
    assert_eq!(
        expected_error_kind("WebSocket connect: HTTP error: 502: upstream returned bad gateway"),
        Some(ExpectedErrorKind::TransientUpstreamHttp)
    );

    // Trailing-newline separator (multi-line error chain).
    assert_eq!(
        expected_error_kind("WebSocket connect: HTTP error: 504\nupstream gateway"),
        Some(ExpectedErrorKind::TransientUpstreamHttp)
    );
}

#[test]
fn does_not_classify_unrelated_http_error_text_as_transient_socket() {
    // Bare numeric "HTTP error: 5023" (port number, runbook ID) without
    // a separator must NOT silence — pin the matcher to space/newline/colon.
    assert_eq!(expected_error_kind("HTTP error: 5023"), None);
    // Non-transient HTTP statuses must not match — `WsError::Http` for
    // a 401 / 403 / 404 is genuinely actionable (auth / routing bug).
    for raw in [
        "WebSocket connect: HTTP error: 401 Unauthorized",
        "WebSocket connect: HTTP error: 403 Forbidden",
        "WebSocket connect: HTTP error: 404 Not Found",
        "WebSocket connect: HTTP error: 500 Internal Server Error",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            None,
            "must NOT silence actionable socket HTTP error: {raw}"
        );
    }
}

#[test]
fn does_not_classify_actionable_provider_errors_as_transient_upstream() {
    // 4xx (other than 408/429) and non-transient 5xx must continue to
    // reach Sentry — those are real bugs (wrong model name, malformed
    // request, internal exception) that need to be triaged.
    for raw in [
        "OpenAI API error (400): bad request",
        "OpenAI API error (401): unauthorized",
        "OpenAI API error (403): forbidden",
        "OpenAI API error (404): model not found",
        "OpenAI API error (500): internal server error",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            None,
            "must NOT silence actionable provider error: {raw}"
        );
    }

    // A free-form message that merely mentions "504" without the
    // `api error (` prefix must not be classified — pin the match to
    // the canonical shape from `ops::api_error`.
    assert_eq!(
        expected_error_kind("see runbook for 504 handling at https://example.com/504"),
        None
    );
}

#[test]
fn classifies_backend_user_error_responses() {
    // OPENHUMAN-TAURI-BC: SharePoint authorize 400 because the user
    // didn't fill in the required Tenant Name field. After the
    // ProviderUserState classifier was added (#1472 wave E), this
    // canonical shape now lands in the more specific
    // ProviderUserState bucket — `"missing required fields"` wins
    // over the generic 4xx matcher. Either expected-kind silences
    // Sentry; the dedicated bucket gives operators a finer-grained
    // `kind="provider_user_state"` info-log facet for triage.
    let bc = "Backend returned 400 Bad Request for POST \
              https://api.tinyhumans.ai/agent-integrations/composio/authorize: \
              Composio authorization failed: 400 \
              {\"error\":{\"message\":\"Missing required fields: Tenant Name\",\
              \"slug\":\"ConnectedAccount_MissingRequiredFields\",\"status\":400}}";
    assert_eq!(
        expected_error_kind(bc),
        Some(ExpectedErrorKind::ProviderUserState),
        "OPENHUMAN-TAURI-BC wire shape must classify as ProviderUserState (the \
         more specific bucket once #1472 wave E added it)"
    );

    // Cover the rest of the 4xx surface produced by integrations /
    // composio clients — all user-input / auth-state failures that
    // Sentry can't action.
    for raw in [
        "Backend returned 400 Bad Request for POST https://api.example.com/x: bad input",
        "Backend returned 401 Unauthorized for GET https://api.example.com/x: token expired",
        "Backend returned 403 Forbidden for GET https://api.example.com/x: permission denied",
        "Backend returned 404 Not Found for GET https://api.example.com/x: missing",
        "Backend returned 422 Unprocessable Entity for POST https://api.example.com/x: validation failed",
        "Backend returned 451 Unavailable for Legal Reasons for GET https://api.example.com/x: blocked",
        // Lowercased context wrapping is irrelevant — substring match is case-insensitive.
        "[observability] integrations.post failed: Backend returned 400 Bad Request for POST https://api.tinyhumans.ai/x: detail",
    ] {
        assert_eq!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::BackendUserError),
            "must classify as backend user-error: {raw}"
        );
    }
}

#[test]
fn tool_execute_backend_401_invalid_token_does_not_hard_report() {
    // TAURI-RUST-84E: the integrations client demotes its backend 401 via
    // `report_error_or_expected`, but ALSO `anyhow::bail!`s it. The error
    // bubbles to the agent tool-execute loop
    // (`agent::harness::engine::tools::run_one_tool`'s `Ok(Err(e))` arm),
    // which previously called the unconditional `report_error` — a second,
    // hard Sentry event (domain=tool / operation=execute) for an
    // already-classified user-end invalid-token condition. The arm now
    // routes through `report_error_or_expected`; this test pins the exact
    // wire shape so a classifier regression that lets the 401 escape (and
    // resume double-reporting) fails CI.
    //
    // Mirror the tool-execute call path: the integrations client bails with
    // `anyhow::anyhow!("Backend returned {status} for POST {url}: {detail}")`,
    // the agent arm renders it with `{e:#}`, then `report_error_or_expected`
    // consults `expected_error_kind`. Assert that classifier returns the
    // expected user-state bucket (so the report is demoted, not hard).
    let e = anyhow::anyhow!(
        "Backend returned 401 Unauthorized for POST \
         https://api.tinyhumans.ai/agent-integrations/parallel/search: Invalid token"
    );
    assert_eq!(
        expected_error_kind(&format!("{e:#}")),
        Some(ExpectedErrorKind::BackendUserError),
        "the 84E tool-execute backend-401 wire shape must classify as expected \
         user-state so report_error_or_expected demotes it instead of firing a \
         hard tool/execute Sentry event"
    );

    // A genuine tool failure (no classifier arm) must keep surfacing as a
    // hard error — confirm routing ALL tool errors through
    // `report_error_or_expected` does not silently swallow real failures.
    assert_eq!(
        expected_error_kind("tool 'web_search' panicked: index out of bounds"),
        None,
        "a genuine tool failure must NOT be classified as expected — it still \
         reaches Sentry as a hard error"
    );
}
