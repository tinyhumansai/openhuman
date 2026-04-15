//! Parse screenshot tool stdout (saved path / data URLs) and write decoded images.

use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

pub fn extract_data_url(raw: &str) -> Option<String> {
    raw.lines().find_map(|line| {
        let trimmed = line.trim();
        trimmed
            .starts_with("data:image/")
            .then(|| trimmed.to_string())
    })
}

pub fn extract_saved_path(raw: &str) -> Option<PathBuf> {
    const PREFIX: &str = "Screenshot saved to: ";
    raw.lines()
        .find_map(|line| line.strip_prefix(PREFIX).map(PathBuf::from))
}

pub fn decode_data_url_bytes(data_url: &str) -> Result<Vec<u8>, String> {
    let (meta, payload) = data_url
        .split_once(',')
        .ok_or_else(|| "invalid data URL: missing comma separator".to_string())?;
    if !meta.starts_with("data:image/") || !meta.ends_with(";base64") {
        return Err("invalid data URL: expected data:image/*;base64,...".to_string());
    }
    BASE64_STANDARD
        .decode(payload)
        .map_err(|e| format!("failed to decode base64 image payload: {e}"))
}

pub fn write_bytes_to_path(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create output directory: {e}"))?;
        }
    }
    std::fs::write(path, bytes).map_err(|e| format!("failed to write output file: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const TINY_PNG_B64: &str =
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

    #[test]
    fn extract_data_url_finds_data_url_line() {
        let raw = format!("some header\ndata:image/png;base64,{TINY_PNG_B64}\nsome footer");
        let result = extract_data_url(&raw);
        assert!(result.is_some());
        assert!(result.unwrap().starts_with("data:image/png;base64,"));
    }

    #[test]
    fn extract_data_url_returns_none_when_absent() {
        assert!(extract_data_url("no data url here").is_none());
    }

    #[test]
    fn extract_saved_path_parses_prefix() {
        let raw = "Screenshot saved to: /tmp/shot.png";
        let path = extract_saved_path(raw).unwrap();
        assert_eq!(path, PathBuf::from("/tmp/shot.png"));
    }

    #[test]
    fn extract_saved_path_returns_none_when_absent() {
        assert!(extract_saved_path("nothing useful").is_none());
    }

    #[test]
    fn decode_data_url_bytes_decodes_valid_png() {
        let data_url = format!("data:image/png;base64,{TINY_PNG_B64}");
        let bytes = decode_data_url_bytes(&data_url).unwrap();
        // PNG magic bytes
        assert_eq!(&bytes[0..4], b"\x89PNG");
    }

    #[test]
    fn decode_data_url_bytes_rejects_missing_comma() {
        let err = decode_data_url_bytes("data:image/png;base64").unwrap_err();
        assert!(err.contains("missing comma"));
    }

    #[test]
    fn decode_data_url_bytes_rejects_wrong_prefix() {
        let err = decode_data_url_bytes("data:text/plain;base64,aGVsbG8=").unwrap_err();
        assert!(err.contains("invalid data URL"));
    }

    #[test]
    fn write_bytes_to_path_creates_file() {
        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("out.png");
        write_bytes_to_path(&dest, b"hello").unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
    }

    #[test]
    fn write_bytes_to_path_creates_parent_dirs() {
        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("sub/dir/out.png");
        write_bytes_to_path(&dest, b"data").unwrap();
        assert!(dest.exists());
    }
}
