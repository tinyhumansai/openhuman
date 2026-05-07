//! Pure helpers for the `meet` domain.
//!
//! Validation lives here so it can be unit-tested without standing up the
//! full RPC machinery.

/// Validate that a string is a Google Meet call URL we're willing to hand
/// to the embedded webview.
///
/// We accept:
///  - `https://meet.google.com/<code>` where `<code>` looks like a Meet
///    meeting code (three lowercase-letter groups separated by `-`).
///  - `https://meet.google.com/lookup/<id>` (Calendar deep links).
///
/// We reject any other host or scheme to keep the surface small — this
/// RPC is *not* a generic "open any URL in CEF" entrypoint.
pub fn validate_meet_url(raw: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(raw.trim()).map_err(|e| format!("invalid meet_url: {e}"))?;

    if url.scheme() != "https" {
        return Err(format!(
            "invalid meet_url: scheme `{}` not allowed (only https)",
            url.scheme()
        ));
    }

    let host = url
        .host_str()
        .ok_or_else(|| "invalid meet_url: missing host".to_string())?;
    if host != "meet.google.com" {
        return Err(format!(
            "invalid meet_url: host `{host}` not allowed (only meet.google.com)"
        ));
    }

    let path = url.path().trim_matches('/');
    let allowed_path = is_meet_code(path) || path.starts_with("lookup/");
    if !allowed_path {
        return Err(format!(
            "invalid meet_url: path `/{path}` is not a Meet meeting code or lookup link"
        ));
    }

    Ok(url)
}

/// Trim and validate the display name. Meet's "Your name" field accepts a
/// wide range, but we cap length to a sane value so a malformed payload
/// can't push a 10MB string into the webview.
pub fn validate_display_name(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("display_name must not be empty".into());
    }
    if trimmed.chars().count() > 64 {
        return Err("display_name exceeds 64 characters".into());
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err("display_name contains control characters".into());
    }
    Ok(trimmed.to_string())
}

fn is_meet_code(path: &str) -> bool {
    let parts: Vec<&str> = path.split('-').collect();
    if parts.len() != 3 {
        return false;
    }
    let lengths_ok = parts[0].len() >= 3 && parts[1].len() >= 3 && parts[2].len() >= 3;
    let alpha_only = parts
        .iter()
        .all(|p| p.chars().all(|c| c.is_ascii_lowercase()));
    lengths_ok && alpha_only
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_meet_code_url() {
        let u = validate_meet_url("https://meet.google.com/abc-defg-hij").unwrap();
        assert_eq!(u.host_str(), Some("meet.google.com"));
    }

    #[test]
    fn accepts_lookup_url() {
        validate_meet_url("https://meet.google.com/lookup/abcdef1234").unwrap();
    }

    #[test]
    fn rejects_http_scheme() {
        assert!(validate_meet_url("http://meet.google.com/abc-defg-hij").is_err());
    }

    #[test]
    fn rejects_other_hosts() {
        assert!(validate_meet_url("https://example.com/abc-defg-hij").is_err());
        assert!(validate_meet_url("https://meet.google.evil.com/abc-defg-hij").is_err());
    }

    #[test]
    fn rejects_nonsense_paths() {
        assert!(validate_meet_url("https://meet.google.com/").is_err());
        assert!(validate_meet_url("https://meet.google.com/foo").is_err());
        assert!(validate_meet_url("https://meet.google.com/AB-CD-EF").is_err());
    }

    #[test]
    fn trims_and_validates_display_name() {
        assert_eq!(validate_display_name("  Alice  ").unwrap(), "Alice");
        assert!(validate_display_name("").is_err());
        assert!(validate_display_name("   ").is_err());
        assert!(validate_display_name(&"x".repeat(65)).is_err());
        assert!(validate_display_name("hi\nthere").is_err());
    }
}
