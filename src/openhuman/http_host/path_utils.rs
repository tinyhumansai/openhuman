use std::path::{Component, Path, PathBuf};

pub(crate) fn canonicalize_hosted_directory(input: &str) -> Result<PathBuf, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("directory must not be empty".to_string());
    }
    let path = PathBuf::from(trimmed);
    let canonical = std::fs::canonicalize(&path).map_err(|e| {
        format!(
            "failed to resolve hosted directory '{}': {e}",
            path.display()
        )
    })?;
    let metadata = std::fs::metadata(&canonical).map_err(|e| {
        format!(
            "failed to read hosted directory '{}': {e}",
            canonical.display()
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!(
            "hosted path is not a directory: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

pub(crate) fn sanitize_bind_host(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("bind_host must not be empty".to_string());
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err("bind_host must be a hostname or IP address, not a path".to_string());
    }
    Ok(trimmed.to_string())
}

pub(crate) fn sanitize_optional_label(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(120).collect())
}

pub(crate) fn render_host_for_url(bind_host: &str) -> String {
    if bind_host.contains(':') && !bind_host.starts_with('[') {
        format!("[{bind_host}]")
    } else {
        bind_host.to_string()
    }
}

pub(crate) fn resolve_request_path(
    root_dir: &Path,
    requested_path: &str,
) -> Result<PathBuf, String> {
    let mut candidate = root_dir.to_path_buf();
    let trimmed = requested_path.trim_matches('/');
    if trimmed.is_empty() {
        return Ok(candidate);
    }

    for segment in trimmed.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        let decoded = urlencoding::decode(segment)
            .map_err(|e| format!("invalid URL path segment '{}': {e}", segment))?;
        if decoded.contains('/') || decoded.contains('\\') || decoded.contains('\0') {
            return Err(format!("invalid path segment '{}'", decoded));
        }
        let path_component = Path::new(decoded.as_ref());
        if path_component.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(format!("path traversal is not allowed: '{}'", decoded));
        }
        candidate.push(path_component);
    }

    if candidate.exists() {
        let canonical = std::fs::canonicalize(&candidate).map_err(|e| {
            format!(
                "failed to resolve requested path '{}': {e}",
                candidate.display()
            )
        })?;
        if !canonical.starts_with(root_dir) {
            return Err("requested path escapes hosted directory".to_string());
        }
        Ok(canonical)
    } else {
        Ok(candidate)
    }
}

pub(crate) fn parent_href_for(requested_path: &str) -> String {
    let trimmed = requested_path.trim_matches('/');
    let mut parts = trimmed
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let _ = parts.pop();
    if parts.is_empty() {
        "/".to_string()
    } else {
        format!("/{}/", parts.join("/"))
    }
}

pub(crate) fn child_href_for(requested_path: &str, child_name: &str, suffix: &str) -> String {
    let encoded = urlencoding::encode(child_name);
    let trimmed = requested_path.trim_matches('/');
    if trimmed.is_empty() {
        format!("/{encoded}{suffix}")
    } else {
        format!("/{trimmed}/{encoded}{suffix}")
    }
}

pub(crate) fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(crate) fn content_type_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") | Some("mjs") => "application/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("txt") | Some("log") | Some("md") => "text/plain; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("pdf") => "application/pdf",
        Some("csv") => "text/csv; charset=utf-8",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}

pub(crate) fn redact_path_for_log(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|segment| segment.to_str())
        .filter(|segment| !segment.is_empty())
        .unwrap_or("<root>");
    format!("<redacted>/{name}")
}
