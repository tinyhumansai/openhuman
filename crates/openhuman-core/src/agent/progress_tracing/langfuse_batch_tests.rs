use super::*;

// The canonical production and staging APIs accept traces. Unknown origins
// must remain closed before a session bearer is read.

#[test]
fn push_is_allowed_for_our_backends() {
    assert!(push_allowed("production"));
    assert!(push_allowed("staging"));
    assert!(push_allowed("development"));
    assert!(!push_allowed("external"));
}

#[test]
fn an_unrecognised_environment_fails_closed() {
    // The allowlist, not a `!= "production"` negation, is what makes this
    // true: a bucket nobody has thought of yet does not push.
    for unknown in ["preview", "prod", "PRODUCTION", "", "qa", "external"] {
        assert!(
            !push_allowed(unknown),
            "{unknown:?} must not push — the allowlist is the fail-closed guard"
        );
    }
}

/// A config as the installed host's enricher resolves it for a hosted
/// control plane in `environment`.
fn config_resolved_as(environment: &str) -> Config {
    let mut config = Config::default();
    config.resolved_endpoints.deployment_environment = Some(environment.to_string());
    config
}

#[test]
fn every_environment_for_base_bucket_is_classified_deliberately() {
    // Ties the two functions together: if `environment_for_base` grows a
    // bucket, this fails until someone decides which side it belongs on.
    for (resolved, base) in [
        ("staging", "https://staging.example.test"),
        ("production", "https://api.example.test"),
    ] {
        let config = config_resolved_as(resolved);
        assert!(push_allowed(environment_for_base(&config, base)));
    }
    assert!(push_allowed(environment_for_base(
        &Config::default(),
        "http://localhost:7788"
    )));
}

#[test]
fn otlp_url_is_the_host_resolved_tracing_endpoint() {
    let mut config = Config::default();
    assert_eq!(otlp_url(&config), "", "nothing resolved means no push");
    config.resolved_endpoints.tracing_otlp_url =
        Some("https://api.example.test/traces/otel/v1/traces".to_string());
    assert_eq!(
        otlp_url(&config),
        "https://api.example.test/traces/otel/v1/traces"
    );
}

#[test]
fn environment_derivation_follows_the_resolved_bucket() {
    let staging = config_resolved_as("staging");
    let prod = config_resolved_as("production");
    assert_eq!(
        environment_for_base(&staging, "https://staging.example.test"),
        "staging"
    );
    assert_eq!(
        environment_for_base(&prod, "https://api.example.test"),
        "production"
    );
    for local in ["http://localhost:5000", "http://127.0.0.1:5000"] {
        assert_eq!(
            environment_for_base(&Config::default(), local),
            "development"
        );
    }
}

/// With no host registered nothing resolves a hosted bucket, so every
/// non-local origin is external and does not push.
#[test]
fn without_a_registered_host_every_remote_origin_is_external() {
    let config = Config::default();
    for base in ["https://api.example.test", "https://staging.example.test"] {
        assert_eq!(environment_for_base(&config, base), "external");
    }
}

/// A plaintext or unrecognised bucket never pushes, even when the host
/// resolved a hosted bucket for its own control plane.
#[test]
fn a_resolved_bucket_does_not_unlock_plaintext_or_unknown_buckets() {
    let prod = config_resolved_as("production");
    assert_eq!(
        environment_for_base(&prod, "http://api.example.test"),
        "external"
    );
    assert_eq!(
        environment_for_base(&config_resolved_as("preview"), "https://api.example.test"),
        "external"
    );
}

/// The local buckets the substring form missed. An IPv6 loopback backend
/// is an ordinary local setup, and before the parse it classified as
/// external — so turning the push gate on would have silently stopped
/// exports that had been working.
#[test]
fn local_backends_are_development_including_ipv6_and_private_ranges() {
    for base in [
        "http://[::1]:7788",
        "http://[0:0:0:0:0:0:0:1]:7788",
        "http://[::]:7788",
        "http://192.168.1.20:5000",
        "http://10.0.0.5:5000",
        "http://api.localhost:5000",
        "http://0.0.0.0:5000",
    ] {
        let environment = environment_for_base(&Config::default(), base);
        assert_eq!(
            environment, "development",
            "{base} must classify as development, got {environment}"
        );
        assert!(push_allowed(environment), "{base} must stay pushable");
    }
}
