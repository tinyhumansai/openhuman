use super::*;

#[test]
fn release_tag_uses_the_crate_version() {
    let tag = process::sentry::release_tag(env!("CARGO_PKG_VERSION"), None);
    assert_eq!(tag, format!("openhuman@{}", env!("CARGO_PKG_VERSION")));
}

#[test]
fn the_shared_chain_scrubs_secrets() {
    // The CLI no longer carries its own scrubber; this pins that the one it
    // installs (embed's) still redacts the shapes the old one did.
    assert_eq!(
        process::sentry::scrub_secrets("Authorization: Bearer abc123xyz"),
        "Authorization: Bearer [REDACTED]"
    );
    assert_eq!(
        process::sentry::scrub_secrets("api_key=sk-abc123"),
        "api_key=[REDACTED]"
    );
}
