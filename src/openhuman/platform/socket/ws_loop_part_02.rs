
// ---------------------------------------------------------------------------
// Message handling
// ---------------------------------------------------------------------------

/// Handle an incoming Engine.IO text message by its type prefix.
fn handle_eio_message(
    text: &str,
    emit_tx: &mpsc::UnboundedSender<String>,
    shared: &Arc<SharedState>,
) {
    if text.is_empty() {
        return;
    }

    match text.as_bytes()[0] {
        b'2' => {
            // Engine.IO PING → respond with PONG
            let _ = emit_tx.send("3".to_string());
        }
        b'3' => {
            // Engine.IO PONG — ignore (server responding to our ping)
        }
        b'4' => {
            // Engine.IO MESSAGE → contains Socket.IO packet
            if text.len() > 1 {
                handle_sio_packet(&text[1..], emit_tx, shared);
            }
        }
        b'1' => {
            log::info!("[socket] Engine.IO CLOSE from server");
        }
        b'6' => {
            // Engine.IO NOOP
        }
        _ => {
            log::debug!(
                "[socket] Unknown EIO packet: {}",
                utf8_safe_prefix_at_byte_boundary(text, 30)
            );
        }
    }
}

/// Handle a Socket.IO packet (after stripping the Engine.IO '4' prefix).
fn handle_sio_packet(
    text: &str,
    emit_tx: &mpsc::UnboundedSender<String>,
    shared: &Arc<SharedState>,
) {
    if text.is_empty() {
        return;
    }

    match text.as_bytes()[0] {
        b'2' => {
            // Socket.IO EVENT: 2["eventName", data]
            if let Some((event_name, data)) = parse_sio_event(&text[1..]) {
                handle_sio_event(&event_name, data, emit_tx, shared);
            } else {
                log::warn!(
                    "[socket] Failed to parse SIO EVENT: {}",
                    utf8_safe_prefix_at_byte_boundary(text, 80)
                );
            }
        }
        b'3' => {
            // Socket.IO ACK: 3<ackId>[ackPayload]
            if let Some((ack_id, data)) = parse_sio_ack(&text[1..]) {
                if shared.ack_registry.resolve(ack_id, data) {
                    log::debug!("[socket] SIO ACK resolved ack_id={ack_id}");
                } else {
                    log::warn!("[socket] SIO ACK had no pending waiter ack_id={ack_id}");
                }
            } else {
                log::warn!(
                    "[socket] Failed to parse SIO ACK: {}",
                    utf8_safe_prefix_at_byte_boundary(text, 80)
                );
            }
        }
        b'0' => {
            // Socket.IO CONNECT (re-ack during reconnection) — update sid
            log::debug!("[socket] SIO CONNECT re-ack");
            if text.len() > 1 {
                if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text[1..]) {
                    if let Some(sid) = data.get("sid").and_then(|v| v.as_str()) {
                        *shared.socket_id.write() = Some(sid.to_string());
                        emit_state_change(shared);
                    }
                }
            }
        }
        b'1' => {
            // Socket.IO DISCONNECT
            log::info!("[socket] SIO DISCONNECT from server");
            super::medulla::workflows::end_connection_generation();
            *shared.status.write() = ConnectionStatus::Disconnected;
            *shared.socket_id.write() = None;
            emit_state_change(shared);
        }
        b'4' => {
            // Socket.IO CONNECT_ERROR
            let error_str = if text.len() > 1 {
                &text[1..]
            } else {
                "unknown"
            };
            log::error!("[socket] SIO CONNECT_ERROR: {}", error_str);
        }
        _ => {
            log::debug!(
                "[socket] Unknown SIO packet type: {}",
                utf8_safe_prefix_at_byte_boundary(text, 30)
            );
        }
    }
}

fn parse_sio_ack(text: &str) -> Option<(u64, serde_json::Value)> {
    let json_start = text.find('[')?;
    if json_start == 0 {
        return None;
    }
    let ack_id = text[..json_start].parse::<u64>().ok()?;
    let mut args: Vec<serde_json::Value> = serde_json::from_str(&text[json_start..]).ok()?;
    let data = if args.len() == 1 {
        args.pop().unwrap_or(serde_json::Value::Null)
    } else {
        serde_json::Value::Array(args)
    };
    Some((ack_id, data))
}

// ---------------------------------------------------------------------------
// Redirect-following connect
// ---------------------------------------------------------------------------

/// Connect to `ws_url`, following HTTP 3xx redirects up to `MAX_REDIRECT_HOPS`.
///
/// Plain `connect_async` returns an error on any non-`101 Switching Protocols`
/// response, so a Cloudflare-style `http://… → https://…` 301 (which happens
/// whenever `BACKEND_URL` is configured without TLS) used to be fatal — the
/// reconnect loop would hammer the same dead URL forever at error level.
///
/// On each redirect we:
///   1. resolve the `Location` header against the current URL (handles relative
///      Location values),
///   2. upgrade the scheme so the next attempt is still a WebSocket
///      (`http` → `ws`, `https` → `wss`; `ws`/`wss` pass through),
///   3. mutate `ws_url` in place so the redirect target is pinned for
///      subsequent reconnects (no need to re-hit the redirect every retry),
///   4. record a one-shot warning in `SharedState.error` the first time we
///      follow a redirect so the UI can surface "your `BACKEND_URL` is stale".
///
/// On non-redirect failures the original error is returned and the caller
/// counts it toward the exponential backoff like before.
async fn connect_with_redirects(
    ws_url: &mut String,
    shared: &Arc<SharedState>,
) -> Result<WsStream, WsError> {
    let original = ws_url.clone();
    for hop in 0..=MAX_REDIRECT_HOPS {
        match connect_async(ws_url.as_str()).await {
            Ok((stream, _response)) => return Ok(stream),
            Err(WsError::Http(response)) if is_redirect_status(response.status()) => {
                if hop == MAX_REDIRECT_HOPS {
                    log::error!(
                        "[socket] Exceeded {MAX_REDIRECT_HOPS} redirect hops starting from {original}; giving up"
                    );
                    return Err(WsError::Http(response));
                }
                let location = match extract_location_header(&response) {
                    Some(loc) => loc,
                    None => {
                        log::error!(
                            "[socket] Redirect {} from {ws_url} missing Location header",
                            response.status()
                        );
                        return Err(WsError::Http(response));
                    }
                };
                let next_url = match resolve_redirect_target(ws_url, &location) {
                    Ok(url) => url,
                    Err(e) => {
                        log::error!(
                            "[socket] Cannot follow redirect to {location} from {ws_url}: {e}"
                        );
                        return Err(WsError::Http(response));
                    }
                };
                log::warn!(
                    "[socket] Server redirected ({}) {} → {}",
                    response.status(),
                    ws_url,
                    next_url
                );
                // Only persist a stale-BACKEND_URL warning for permanent
                // redirects (301 / 308). Temporary redirects (302 / 307) say
                // "this time, go elsewhere" — the configured BACKEND_URL is
                // still correct, and surfacing a "please update config" hint
                // for a transient hop would be misleading. Per CodeRabbit
                // review on PR #1547.
                if matches!(
                    response.status(),
                    StatusCode::MOVED_PERMANENTLY | StatusCode::PERMANENT_REDIRECT
                ) {
                    record_redirect_warning(shared, &original, &next_url);
                }
                *ws_url = next_url;
            }
            Err(e) => return Err(e),
        }
    }
    // Unreachable: the loop either returns Ok, returns the redirect error after
    // exhausting hops, or returns a non-redirect Err.
    unreachable!("connect_with_redirects exited loop without returning")
}

/// Statuses we treat as "follow the Location and retry".
///
/// 308 (Permanent Redirect) and 307 (Temporary Redirect) explicitly preserve
/// the method; 301/302 historically do too for upgrade requests in practice.
/// Anything else (300, 304, ...) stays an error.
fn is_redirect_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    )
}

fn extract_location_header(
    response: &tokio_tungstenite::tungstenite::http::Response<Option<Vec<u8>>>,
) -> Option<String> {
    response
        .headers()
        .get(tokio_tungstenite::tungstenite::http::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

/// Resolve `location` against `current_ws_url` and rewrite the scheme so the
/// result is still a valid WebSocket URL.
///
/// `location` may be absolute (`https://host/path?q=1`) or relative
/// (`/socket.io/?EIO=4`). We use the `url` crate's relative-URL parser to do
/// the join the same way browsers do, then map `http`→`ws` / `https`→`wss`.
fn resolve_redirect_target(current_ws_url: &str, location: &str) -> Result<String, String> {
    let base = url::Url::parse(current_ws_url).map_err(|e| format!("invalid current URL: {e}"))?;
    let resolved = base
        .join(location)
        .map_err(|e| format!("invalid Location {location:?}: {e}"))?;

    let upgraded_scheme = match resolved.scheme() {
        "http" => "ws",
        "https" => "wss",
        "ws" | "wss" => resolved.scheme(),
        other => return Err(format!("unsupported scheme in Location: {other}")),
    };

    let mut next = resolved.clone();
    next.set_scheme(upgraded_scheme)
        .map_err(|_| format!("failed to set scheme {upgraded_scheme} on {resolved}"))?;
    Ok(next.to_string())
}

/// Persist a one-shot, user-visible warning that the backend redirected the
/// configured socket URL. Subsequent redirects in the same connect attempt
/// don't overwrite — the first hop carries the actionable signal.
fn record_redirect_warning(shared: &Arc<SharedState>, original: &str, resolved: &str) {
    let mut slot = shared.error.write();
    if slot.is_some() {
        return;
    }
    *slot = Some(format!(
        "Backend redirected {original} → {resolved}. Update BACKEND_URL to the resolved URL to avoid the extra hop."
    ));
}
