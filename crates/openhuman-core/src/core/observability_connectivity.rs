/// Defense-in-depth `before_send` filter for connectivity and network failures.
/// Primary suppression lives at the caller sites; this remains an outer guard
/// for any future call site that bypasses those classifiers.
pub fn is_connectivity_event(event: &sentry::protocol::Event<'_>) -> bool {
    let texts: Vec<&str> = {
        let mut v = Vec::with_capacity(2 + event.exception.values.len());
        if let Some(msg) = event.message.as_deref() {
            v.push(msg);
        }
        if let Some(log) = event.logentry.as_ref().map(|l| l.message.as_str()) {
            v.push(log);
        }
        for exc in &event.exception.values {
            if let Some(val) = exc.value.as_deref() {
                v.push(val);
            }
        }
        v
    };
    if texts.is_empty() {
        return false;
    }

    let lower = texts.join(" ").to_ascii_lowercase();

    // Core HTTP transport errors
    if lower.contains("connection refused")
        || lower.contains("connection reset")
        || lower.contains("connection closed before")
        || lower.contains("broken pipe")
        || lower.contains("tls handshake eof")
    {
        return true;
    }

    // Backend / provider gateway errors
    if lower.contains("502 bad gateway")
        || lower.contains("504 gateway timeout")
        || lower.contains("502 gateway timeout")
        || lower.contains("timeout: 503")
        || lower.contains("upstream connect error")
    {
        return true;
    }

    // CoreRpcError / HTTP 401 (from frontend — user session issue, not code defect)
    // Only match bare HTTP 401 (CoreRpcError / fetch), not llm_provider 401
    // which is handled by the session-expired classifier
    let domain = event.tags.get("domain").map(String::as_str);
    if domain != Some("llm_provider")
        && domain != Some("backend_api")
        && (lower.contains("http 401")
            || lower.contains("status: 401")
            || lower.contains("401 unauthorized"))
    {
        return true;
    }

    // Timeout patterns — scoped to known transient forms so genuine
    // non-transport timeouts (database locks, inference hangs) still
    // reach Sentry for investigation.
    if lower.contains("connection timed out")
        || lower.contains("request timed out")
        || lower.contains("deadline has elapsed")
        || lower.contains("timeout after")
    {
        return true;
    }

    false
}

