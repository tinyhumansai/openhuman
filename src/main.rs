use once_cell::sync::Lazy;
use regex::Regex;

fn main() {
    // Initialize Sentry as the very first operation so the guard outlives everything.
    // If OPENHUMAN_SENTRY_DSN is unset or empty, sentry::init returns a no-op guard.
    let _sentry_guard = sentry::init(sentry::ClientOptions {
        dsn: std::env::var("OPENHUMAN_SENTRY_DSN")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| option_env!("OPENHUMAN_SENTRY_DSN").map(|s| s.to_string()))
            .filter(|s| !s.is_empty())
            .and_then(|s| s.parse().ok()),
        release: Some(std::borrow::Cow::Borrowed(env!("CARGO_PKG_VERSION"))),
        environment: Some(if cfg!(debug_assertions) {
            "development".into()
        } else {
            "production".into()
        }),
        send_default_pii: false,
        before_send: Some(std::sync::Arc::new(|mut event| {
            // Strip server_name (hostname) to avoid leaking machine identity
            event.server_name = None;
            // Strip user context entirely
            event.user = None;
            // Scrub exception messages for secrets
            for exc in &mut event.exception.values {
                if let Some(ref value) = exc.value {
                    exc.value = Some(scrub_secrets(value));
                }
            }
            Some(event)
        })),
        sample_rate: 1.0,
        ..sentry::ClientOptions::default()
    });

    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(err) = openhuman_core::run_core_from_args(&args) {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Secret scrubbing
// ---------------------------------------------------------------------------

static SECRET_PATTERNS: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    vec![
        (Regex::new(r"(?i)(bearer\s+)\S+").unwrap(), "${1}[REDACTED]"),
        (
            Regex::new(r"(?i)(api[_-]?key[=:\s]+)\S+").unwrap(),
            "${1}[REDACTED]",
        ),
        (
            Regex::new(r"(?i)(token[=:\s]+)\S+").unwrap(),
            "${1}[REDACTED]",
        ),
        (Regex::new(r"sk-[a-zA-Z0-9]{20,}").unwrap(), "[REDACTED]"),
    ]
});

/// Replace patterns that look like secrets with `[REDACTED]`.
fn scrub_secrets(input: &str) -> String {
    let mut result = input.to_string();
    for (re, replacement) in SECRET_PATTERNS.iter() {
        result = re.replace_all(&result, *replacement).into_owned();
    }
    result
}
