/// Parses a Sentry release tag like `"openhuman@0.58.0"` or
/// `"openhuman@0.58.0+abc123def456"` into `(major, minor)`.
pub(super) fn parse_release_tag(tag: &str) -> Option<(u32, u32)> {
    let after_at = tag.split('@').nth(1)?;
    let version = after_at.split('+').next()?;
    parse_version(version)
}

/// Parses a `"MAJOR.MINOR.PATCH"` version string into `(major, minor)`.
pub(super) fn parse_version(version: &str) -> Option<(u32, u32)> {
    let mut parts = version.splitn(3, '.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next()?.parse::<u32>().ok()?;
    Some((major, minor))
}

/// Filter out events from **stale releases** — releases older than
/// `MAX_AGE_MINOR_VERSIONS` minor versions behind the current build.
///
/// Ancient-client errors are not actionable against the current codebase.
/// Target: ~4 issues from releases like v0.54.0 against current v0.58+.
#[cfg(feature = "crash-reporting")]
pub fn is_stale_release_event(event: &sentry::protocol::Event<'_>) -> bool {
    // Maximum number of minor versions behind the current release to accept.
    // Events from clients on releases older than this are dropped.
    const MAX_AGE_MINOR_VERSIONS: u32 = 6;

    let Some(release) = event.release.as_ref() else {
        return false;
    };

    let current = env!("CARGO_PKG_VERSION");
    parse_version(current).is_some_and(|(current_major, current_minor)| {
        parse_release_tag(release).is_some_and(|(event_major, event_minor)| {
            if event_major != current_major {
                // Different major version = definitely stale (or from the future)
                return event_major < current_major;
            }
            // Same major: check minor version gap
            current_minor.saturating_sub(event_minor) > MAX_AGE_MINOR_VERSIONS
        })
    })
}
