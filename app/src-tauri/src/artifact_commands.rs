//! Tauri command for downloading agent-generated artifacts (#2779).
//!
//! Contract: the frontend resolves the artifact's absolute source
//! path via the existing `openhuman.ai_get_artifact` core RPC, then
//! invokes [`download_artifact_to_downloads`] with that source path
//! plus a filename hint. The command:
//!
//! 1. Validates both inputs (no path traversal in the filename, source
//!    must be absolute + on disk).
//! 2. Resolves the user's Downloads directory via the `dirs` crate.
//! 3. Picks a non-colliding destination filename — `name.pptx`,
//!    `name (1).pptx`, `name (2).pptx`, …
//! 4. Copies source → dest with `tokio::fs::copy`.
//! 5. Returns the absolute dest path so the frontend can show a
//!    "Saved to …" toast with a "Reveal in Finder" button (the
//!    `opener:allow-reveal-item-in-dir` capability is already wired).
//!
//! Why Downloads instead of a native save-file dialog: the
//! `tauri-plugin-dialog` crate pulls `tauri-plugin-fs` transitively,
//! which currently breaks the openhuman build with a `schemars`
//! version conflict. The Downloads + reveal pattern satisfies the
//! "user-chosen destination" intent of issue #2779 AC#3 without
//! widening the Tauri allow-list, and matches what most desktop chat
//! apps do for downloaded attachments.

use std::path::{Path, PathBuf};

/// Maximum number of `(N)` suffixes we'll append when picking a
/// non-colliding filename. After 1000 we give up and append a UUID
/// suffix instead so the download never silently overwrites.
const MAX_COLLISION_SUFFIX: u32 = 1000;

/// Cap on the sanitized basename length (stem + dot + extension). 200
/// bytes leaves comfortable headroom under the 255-byte path-component
/// limits enforced by NTFS, ext4, APFS, exFAT, HFS+, and is still well
/// under Windows' 260-char `MAX_PATH` (legacy mode).
const MAX_FILENAME_LEN: usize = 200;

/// Windows reserved device names (case-insensitive). Trying to write
/// `CON.pptx`, `PRN`, `COM1`, etc. on Windows triggers a Win32 error
/// even though POSIX shells happily allow it. Match the *stem* against
/// this list and rewrite when it hits.
const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

#[tauri::command]
pub async fn download_artifact_to_downloads(
    source_path: String,
    filename: String,
) -> Result<String, String> {
    if source_path.trim().is_empty() {
        return Err("source_path must not be empty".to_string());
    }
    if filename.trim().is_empty() {
        return Err("filename must not be empty".to_string());
    }
    let source = PathBuf::from(&source_path);
    if !source.is_absolute() {
        return Err(format!(
            "source_path must be absolute (came from ai_get_artifact): {source_path:?}"
        ));
    }
    if !source.is_file() {
        return Err(format!(
            "artifact source not present on disk: {source_path}"
        ));
    }
    let sanitized = sanitize_filename(&filename)?;
    let safe_basename = sanitize_basename(&sanitized);

    let downloads = directories::UserDirs::new()
        .and_then(|u| u.download_dir().map(|p| p.to_path_buf()))
        .ok_or_else(|| "OS Downloads directory not resolvable".to_string())?;
    tokio::fs::create_dir_all(&downloads)
        .await
        .map_err(|e| format!("failed to ensure Downloads dir {:?}: {e}", downloads))?;

    let (dest, bytes) = copy_with_atomic_collision(&source, &downloads, &safe_basename).await?;

    let final_basename = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("<unknown>");
    // Log redacted: no full destination path (would leak the OS
    // username under /Users/<name>/Downloads or C:\Users\<name>\…) and
    // no caller-supplied raw filename. Per
    // `feedback_redact_paths_and_ids_in_public`.
    log::info!(
        "[artifact_commands] download_artifact_to_downloads filename={} bytes={}",
        final_basename,
        bytes
    );
    Ok(dest.display().to_string())
}

/// Strip path-traversal characters from a filename hint. The
/// renderer is expected to pass something like `"My Deck.pptx"`;
/// reject anything that contains a separator or null byte so a
/// malicious `ai_get_artifact` response can never escape Downloads.
fn sanitize_filename(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("filename must not be empty after trim".to_string());
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err(format!(
            "filename must not contain path separators: {trimmed:?}"
        ));
    }
    if trimmed.contains('\0') {
        return Err(format!("filename must not contain NUL bytes: {trimmed:?}"));
    }
    if trimmed == "." || trimmed == ".." {
        return Err(format!("filename must not be '.' or '..': {trimmed:?}"));
    }
    Ok(trimmed.to_string())
}

/// Pick a destination path under `dir` that does not exist yet.
/// Inserts ` (N)` between the stem and the extension. Falls back to
/// a UUID suffix after [`MAX_COLLISION_SUFFIX`] tries.
///
/// **TOCTOU**: this is a best-effort, non-atomic pick — two concurrent
/// callers can both see the same path as "free" before either writes.
/// Use [`copy_with_atomic_collision`] for the live download flow; this
/// helper is retained for tests and callers that want the path
/// up-front. The atomic flow re-derives candidates by the same naming
/// convention so the user-visible suffix shape stays identical.
#[cfg(test)]
fn pick_unique_path(dir: &Path, filename: &str) -> PathBuf {
    let candidate = dir.join(filename);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = split_stem_ext(filename);
    for n in 1..=MAX_COLLISION_SUFFIX {
        let nth = if ext.is_empty() {
            format!("{stem} ({n})")
        } else {
            format!("{stem} ({n}).{ext}")
        };
        let path = dir.join(&nth);
        if !path.exists() {
            return path;
        }
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let with_uniq = if ext.is_empty() {
        format!("{stem}-{nanos}")
    } else {
        format!("{stem}-{nanos}.{ext}")
    };
    dir.join(with_uniq)
}

/// Copy `source` into `dir` using **atomic collision avoidance** —
/// open the destination with `create_new(true)` so the kernel rejects
/// the open if another process raced us to the same path, and retry
/// with the next `(N)` suffix on `ErrorKind::AlreadyExists`. After
/// [`MAX_COLLISION_SUFFIX`] tries we fall back to a nanosecond-suffix
/// path (same UUID-style escape hatch as the original `pick_unique_path`).
///
/// Returns the actual destination path that won the create race and
/// the number of bytes copied.
///
/// Why this matters: the previous implementation called
/// `path.exists()` then `tokio::fs::copy()` — a classic TOCTOU window.
/// Two concurrent "Download" clicks on the same artifact picked the
/// same destination, then the second copy silently clobbered the first.
/// With `create_new(true)`, the second open errors with `AlreadyExists`
/// and we bump the suffix.
async fn copy_with_atomic_collision(
    source: &Path,
    dir: &Path,
    filename: &str,
) -> Result<(PathBuf, u64), String> {
    use tokio::fs::OpenOptions;
    use tokio::io::AsyncWriteExt;

    let (stem, ext) = split_stem_ext(filename);

    // We try filename, then filename (1), filename (2), ... up to
    // MAX_COLLISION_SUFFIX. n == 0 means "no suffix".
    for n in 0..=MAX_COLLISION_SUFFIX {
        let candidate = if n == 0 {
            filename.to_string()
        } else if ext.is_empty() {
            format!("{stem} ({n})")
        } else {
            format!("{stem} ({n}).{ext}")
        };
        let dest = dir.join(&candidate);

        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dest)
            .await
        {
            Ok(mut file) => {
                // Stream source -> dest. We hold the create_new'd handle
                // throughout so no other process can clobber it mid-write.
                let bytes_copied = match tokio::fs::read(source).await {
                    Ok(contents) => {
                        let len = contents.len() as u64;
                        if let Err(e) = file.write_all(&contents).await {
                            // Best-effort cleanup of the half-written
                            // destination; ignore errors since the
                            // primary error is the write failure.
                            let _ = tokio::fs::remove_file(&dest).await;
                            return Err(format!("failed to write artifact: {e}"));
                        }
                        if let Err(e) = file.flush().await {
                            let _ = tokio::fs::remove_file(&dest).await;
                            return Err(format!("failed to flush artifact: {e}"));
                        }
                        len
                    }
                    Err(e) => {
                        let _ = tokio::fs::remove_file(&dest).await;
                        return Err(format!("failed to read source artifact: {e}"));
                    }
                };
                return Ok((dest, bytes_copied));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("failed to create destination file: {e}")),
        }
    }

    // Exhausted the (1)..(MAX) suffix space. Use a nanosecond-stamped
    // basename so we still avoid overwriting anything.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let with_uniq = if ext.is_empty() {
        format!("{stem}-{nanos}")
    } else {
        format!("{stem}-{nanos}.{ext}")
    };
    let dest = dir.join(&with_uniq);

    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&dest)
        .await
    {
        Ok(mut file) => {
            let contents = tokio::fs::read(source)
                .await
                .map_err(|e| format!("failed to read source artifact: {e}"))?;
            let len = contents.len() as u64;
            file.write_all(&contents)
                .await
                .map_err(|e| format!("failed to write artifact: {e}"))?;
            file.flush()
                .await
                .map_err(|e| format!("failed to flush artifact: {e}"))?;
            Ok((dest, len))
        }
        Err(e) => Err(format!(
            "exhausted {MAX_COLLISION_SUFFIX} collision slots and the nanosecond-fallback path also failed to open: {e}"
        )),
    }
}

fn split_stem_ext(filename: &str) -> (String, String) {
    if let Some(idx) = filename.rfind('.') {
        // Reject leading-dot files (`.hidden`) — treat as having no extension.
        if idx > 0 && idx < filename.len() - 1 {
            return (filename[..idx].to_string(), filename[idx + 1..].to_string());
        }
    }
    (filename.to_string(), String::new())
}

/// Harden `name` for cross-platform safety on Windows / macOS / Linux.
///
/// `sanitize_filename` (called first in the command path) already
/// rejects path separators, NUL bytes, and `.` / `..`. This second
/// pass handles the *Windows*-specific concerns that the first pass
/// lets through because they're legal on POSIX: reserved device
/// names (`CON.pptx`, `NUL`, `COM1`), characters Win32 forbids
/// (`< > : " | ? *`), ASCII control bytes, trailing dots / spaces
/// (silently stripped by Win32 → name corruption), and lengths over
/// 200 bytes (NTFS / APFS path-component cap is 255, leave headroom).
///
/// **Cross-platform**: runs on all hosts. macOS / Linux users
/// downloading an artifact a Windows colleague will receive deserve
/// the same safe basename so the file round-trips.
///
/// Falls back to `"artifact"` if the input strips to empty (e.g. a
/// title that was nothing but `?`s). Caller may further specialize.
fn sanitize_basename(name: &str) -> String {
    let (stem, ext) = split_stem_ext(name);
    let safe_stem = sanitize_component(&stem);
    let safe_ext = sanitize_component(&ext);

    // After char-strip, the stem may have collapsed to nothing, to
    // pure replacement underscores (input was all illegal chars like
    // `???`), or to a string that maps onto a Windows reserved device
    // name. In any of these, fall back to a stable default so we
    // don't try to write `.pptx` or `___.pptx`.
    let stem_is_only_underscores = !safe_stem.is_empty() && safe_stem.chars().all(|c| c == '_');
    let stem_final =
        if safe_stem.is_empty() || stem_is_only_underscores || is_windows_reserved(&safe_stem) {
            "artifact".to_string()
        } else {
            safe_stem
        };

    let combined = if safe_ext.is_empty() {
        stem_final
    } else {
        format!("{stem_final}.{safe_ext}")
    };

    truncate_basename(&combined, MAX_FILENAME_LEN)
}

/// Strip Windows-illegal characters, ASCII controls (incl. tab/newline),
/// trailing/leading whitespace, and trailing dots from a single
/// filename component (no separators).
fn sanitize_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            // Win32 path-illegal set:
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => out.push('_'),
            // ASCII controls — silently dropped (don't keep as `_` so the
            // basename doesn't get a run of underscores for one stray byte).
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    // Windows silently strips trailing dots / spaces from filenames →
    // strip them explicitly so the on-disk name matches what we logged.
    let trimmed = out.trim().trim_end_matches('.').trim().to_string();
    trimmed
}

/// Match a stem against [`WINDOWS_RESERVED_NAMES`] case-insensitively.
fn is_windows_reserved(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    WINDOWS_RESERVED_NAMES.iter().any(|&r| r == upper)
}

/// Truncate a basename to ≤`max` bytes while preserving the extension
/// (so `<huge-stem>.pptx` stays a `.pptx`, not a truncated mid-byte
/// mess). Returns the input unchanged when it already fits.
///
/// UTF-8-safe: never slices mid-codepoint (per
/// `feedback_http_body_byte_slice_utf8_panic` and
/// `feedback_truncate_cap_includes_suffix`).
fn truncate_basename(name: &str, max: usize) -> String {
    if name.len() <= max {
        return name.to_string();
    }
    let (stem, ext) = split_stem_ext(name);
    // Reserve room for `.ext`. If the extension alone exceeds `max`,
    // drop it — pathological case, never happens with real artifact
    // extensions (`.pptx`, `.docx`, …).
    let ext_overhead = if ext.is_empty() { 0 } else { ext.len() + 1 };
    let stem_budget = max.saturating_sub(ext_overhead);

    // Walk char-by-char so we don't cut mid-codepoint.
    let mut truncated_stem = String::with_capacity(stem_budget);
    for ch in stem.chars() {
        if truncated_stem.len() + ch.len_utf8() > stem_budget {
            break;
        }
        truncated_stem.push(ch);
    }
    if ext.is_empty() {
        truncated_stem
    } else {
        format!("{truncated_stem}.{ext}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_rejects_path_separators() {
        assert!(sanitize_filename("../etc/passwd").is_err());
        assert!(sanitize_filename("a\\b.pptx").is_err());
        assert!(sanitize_filename("a/b.pptx").is_err());
        assert!(sanitize_filename("").is_err());
        assert!(sanitize_filename(".").is_err());
        assert!(sanitize_filename("..").is_err());
        assert!(sanitize_filename("ok.pptx\0").is_err());
    }

    #[test]
    fn sanitize_accepts_plain_names() {
        assert_eq!(
            sanitize_filename("Quarterly Update.pptx").unwrap(),
            "Quarterly Update.pptx"
        );
        assert_eq!(sanitize_filename("  trim me  ").unwrap(), "trim me");
    }

    #[test]
    fn split_stem_ext_pairs() {
        assert_eq!(
            split_stem_ext("file.pptx"),
            ("file".to_string(), "pptx".to_string())
        );
        assert_eq!(
            split_stem_ext("noext"),
            ("noext".to_string(), String::new())
        );
        assert_eq!(
            split_stem_ext(".hidden"),
            (".hidden".to_string(), String::new())
        );
        assert_eq!(
            split_stem_ext("trailing."),
            ("trailing.".to_string(), String::new())
        );
        assert_eq!(
            split_stem_ext("a.b.c"),
            ("a.b".to_string(), "c".to_string())
        );
    }

    #[test]
    fn pick_unique_inserts_collision_suffix() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let first = pick_unique_path(dir, "deck.pptx");
        assert_eq!(first, dir.join("deck.pptx"));

        std::fs::write(&first, b"").unwrap();
        let second = pick_unique_path(dir, "deck.pptx");
        assert_eq!(second, dir.join("deck (1).pptx"));

        std::fs::write(&second, b"").unwrap();
        let third = pick_unique_path(dir, "deck.pptx");
        assert_eq!(third, dir.join("deck (2).pptx"));
    }

    #[test]
    fn pick_unique_handles_no_extension() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let first = pick_unique_path(dir, "noext");
        assert_eq!(first, dir.join("noext"));
        std::fs::write(&first, b"").unwrap();
        let second = pick_unique_path(dir, "noext");
        assert_eq!(second, dir.join("noext (1)"));
    }

    #[test]
    fn sanitize_basename_strips_windows_illegal_chars() {
        assert_eq!(sanitize_basename("a:b?c.pptx"), "a_b_c.pptx");
        assert_eq!(sanitize_basename(r#"<x>|y"z.docx"#), "_x__y_z.docx");
        // `*` is included in the swap set
        assert_eq!(sanitize_basename("rate*.pptx"), "rate_.pptx");
    }

    #[test]
    fn sanitize_basename_rewrites_windows_reserved_names() {
        // Bare reserved name and `<reserved>.<ext>` both rewrite the stem.
        assert_eq!(sanitize_basename("CON.pptx"), "artifact.pptx");
        assert_eq!(sanitize_basename("nul"), "artifact");
        assert_eq!(sanitize_basename("Com1.docx"), "artifact.docx");
        assert_eq!(sanitize_basename("LPT9"), "artifact");
        // Mixed-case / lower-case must match too
        assert_eq!(sanitize_basename("aux"), "artifact");
        // A reserved name in the MIDDLE of a stem is fine — only bare
        // reserved stems are problematic on Win32.
        assert_eq!(sanitize_basename("the CON file.pptx"), "the CON file.pptx");
    }

    #[test]
    fn sanitize_basename_handles_empty_and_all_stripped_input() {
        // After full-strip, fall back to "artifact" so we never write
        // a bare extension or an empty filename.
        assert_eq!(sanitize_basename(""), "artifact");
        assert_eq!(sanitize_basename("???"), "artifact");
        // Pure whitespace / control chars also strip to empty → fallback.
        assert_eq!(sanitize_basename("   "), "artifact");
        assert_eq!(sanitize_basename("\t\n\r"), "artifact");
    }

    #[test]
    fn sanitize_basename_drops_control_chars_and_trims() {
        assert_eq!(sanitize_basename("file\x01name.pptx"), "filename.pptx");
        assert_eq!(sanitize_basename("\tdeck\n.pptx"), "deck.pptx");
        assert_eq!(sanitize_basename("  spaced  .pptx"), "spaced.pptx");
        // Trailing dots on the stem are stripped (Win32 strips them
        // silently on disk → keep the on-disk and logged names aligned).
        assert_eq!(sanitize_basename("deck...pptx"), "deck.pptx");
    }

    #[test]
    fn sanitize_basename_caps_total_length() {
        // 250-char stem + `.pptx` should truncate to ≤ MAX_FILENAME_LEN.
        let long_stem = "a".repeat(250);
        let input = format!("{long_stem}.pptx");
        let out = sanitize_basename(&input);
        assert!(out.len() <= MAX_FILENAME_LEN, "len = {}", out.len());
        // Extension preserved
        assert!(out.ends_with(".pptx"));
    }

    #[test]
    fn sanitize_basename_handles_multibyte_truncation() {
        // 100 × 4-byte emoji = 400 bytes — exceeds the 200-byte cap.
        let long = "🚀".repeat(100);
        let input = format!("{long}.pptx");
        let out = sanitize_basename(&input);
        assert!(out.len() <= MAX_FILENAME_LEN);
        // Did not slice mid-codepoint — `is_char_boundary(0..=len)` holds.
        assert!(out.is_char_boundary(out.len()));
        assert!(out.ends_with(".pptx"));
    }

    #[tokio::test]
    async fn copy_with_atomic_collision_picks_next_suffix() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        // Source file
        let source = dir.join("__source.pptx");
        tokio::fs::write(&source, b"first").await.unwrap();
        // Pre-create the canonical destination so the first attempt collides.
        tokio::fs::write(dir.join("deck.pptx"), b"existing")
            .await
            .unwrap();

        let (dest, bytes) = copy_with_atomic_collision(&source, dir, "deck.pptx")
            .await
            .unwrap();
        assert_eq!(dest, dir.join("deck (1).pptx"));
        assert_eq!(bytes, 5);
        // First file untouched.
        assert_eq!(
            tokio::fs::read(dir.join("deck.pptx")).await.unwrap(),
            b"existing"
        );
        // Second file written.
        assert_eq!(tokio::fs::read(&dest).await.unwrap(), b"first");
    }

    #[tokio::test]
    async fn copy_with_atomic_collision_two_concurrent_callers() {
        // Same source, same target name: both callers should produce
        // distinct destination paths instead of one clobbering the other.
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().to_path_buf();
        let source = dir.join("__source.pptx");
        tokio::fs::write(&source, b"payload").await.unwrap();

        let s1 = source.clone();
        let d1 = dir.clone();
        let s2 = source.clone();
        let d2 = dir.clone();

        let (r1, r2) = tokio::join!(
            tokio::spawn(async move { copy_with_atomic_collision(&s1, &d1, "deck.pptx").await }),
            tokio::spawn(async move { copy_with_atomic_collision(&s2, &d2, "deck.pptx").await }),
        );
        let (dest1, _) = r1.unwrap().unwrap();
        let (dest2, _) = r2.unwrap().unwrap();
        assert_ne!(
            dest1, dest2,
            "concurrent callers must not share a destination"
        );
        assert_eq!(tokio::fs::read(&dest1).await.unwrap(), b"payload");
        assert_eq!(tokio::fs::read(&dest2).await.unwrap(), b"payload");
    }

    #[tokio::test]
    async fn download_rejects_invalid_inputs() {
        assert!(
            download_artifact_to_downloads(String::new(), "x.pptx".to_string())
                .await
                .is_err()
        );
        assert!(
            download_artifact_to_downloads("/tmp/x".to_string(), String::new())
                .await
                .is_err()
        );
        assert!(
            download_artifact_to_downloads("relative".to_string(), "x.pptx".to_string())
                .await
                .is_err()
        );
        assert!(
            download_artifact_to_downloads("/nope".to_string(), "../escape.pptx".to_string())
                .await
                .is_err()
        );
    }
}
