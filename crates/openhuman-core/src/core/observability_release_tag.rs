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
