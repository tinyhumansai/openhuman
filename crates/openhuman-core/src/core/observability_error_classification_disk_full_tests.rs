use super::*;

/// #8508 / Sentry TAURI-RUST-15PJ: a localized Windows `ERROR_DISK_FULL`
/// rendering carries none of the English anchors — only the locale-stable
/// `(os error 112)` suffix — and must still classify as disk-full on Windows.
#[test]
fn classifies_localized_disk_full_by_os_error_code() {
    let windows_cases = [
        // The exact message from the TAURI-RUST-15PJ event (Simplified
        // Chinese Windows 10, 0.64.15).
        "transcript persistence failed: 磁盘空间不足。 (os error 112)",
        // German Windows `FormatMessageW` text for ERROR_DISK_FULL (112).
        "transcript persistence failed: Auf dem Datenträger ist nicht genügend \
         Speicherplatz vorhanden. (os error 112)",
        // Italian ERROR_DISK_FULL.
        "transcript persistence failed: Spazio insufficiente sul disco. (os error 112)",
        // ERROR_HANDLE_DISK_FULL (39), French.
        "transcript persistence failed: Le disque est plein. (os error 39)",
    ];
    for raw in windows_cases {
        let lower = raw.to_ascii_lowercase();
        assert!(is_disk_full_os_code(&lower, true), "windows: {raw}");
        // The same errno numbers mean something else on unix (112 =
        // EHOSTDOWN, 39 = ENOTEMPTY on Linux) — never disk-full there.
        assert!(!is_disk_full_os_code(&lower, false), "unix: {raw}");
    }

    // Unix ENOSPC (28) under a non-English `LC_MESSAGES` (GTK calls
    // `setlocale`, so `strerror_r` can localize on Linux desktops).
    let unix = "transcript persistence failed: Kein Platz mehr auf dem Gerät (os error 28)"
        .to_ascii_lowercase();
    assert!(is_disk_full_os_code(&unix, false));
    // On Windows 28 is ERROR_OUT_OF_PAPER, not disk-full.
    assert!(!is_disk_full_os_code(&unix, true));

    // The platform-correct arm feeds the public classifier and predicate.
    let native = if cfg!(windows) {
        windows_cases[0]
    } else {
        "transcript persistence failed: Kein Platz mehr auf dem Gerät (os error 28)"
    };
    assert!(is_disk_full_error(native));
    assert_eq!(
        expected_error_kind(native),
        Some(ExpectedErrorKind::DiskFull)
    );
}

#[test]
fn os_error_code_anchor_is_exact_and_local_only() {
    for windows in [true, false] {
        // A longer errno that merely starts with the same digits.
        assert!(!is_disk_full_os_code(
            "write failed (os error 1120)",
            windows
        ));
        assert!(!is_disk_full_os_code(
            "write failed (os error 280)",
            windows
        ));
        assert!(!is_disk_full_os_code(
            "write failed (os error 390)",
            windows
        ));
        // Unrelated errnos.
        assert!(!is_disk_full_os_code("access denied (os error 5)", windows));
        // A remote backend body quoting an errno is the server's problem.
        assert!(!is_disk_full_os_code(
            "backend returned 500 internal server error for post /x: (os error 112) (os error 28)",
            windows
        ));
    }
    // Linux EHOSTDOWN / ENOTEMPTY keep reaching Sentry on unix builds.
    if !cfg!(windows) {
        assert!(!is_disk_full_error(
            "connect failed: Host is down (os error 112)"
        ));
        assert!(!is_disk_full_error(
            "remove_dir failed: Directory not empty (os error 39)"
        ));
    }
}

/// A server whose own disk is full is operator-actionable: a remote response
/// quoting any disk-full phrase or errno must keep reaching Sentry. The shared
/// predicate (used by the `before_send` filter and the report cooldown) and the
/// `DiskFull` classifier arm both reject the backend and embeddings envelopes
/// before any disk-full rule (CodeRabbit on #8517).
#[test]
fn remote_response_envelopes_are_never_local_disk_full() {
    let remote = [
        "Backend returned 500 Internal Server Error for POST https://api.example.test/x: \
         No space left on device (os error 28)",
        "Backend returned 507 Insufficient Storage for PUT https://api.example.test/x: \
         There is not enough space on the disk. (os error 112)",
        "Backend returned 500 Internal Server Error for POST https://api.example.test/x: \
         StorageFull",
        "Embedding API error (500 Internal Server Error): No space left on device (os error 28)",
        "Embedding API error (500 Internal Server Error): disk full (os error 112) (os error 39)",
    ];
    for raw in remote {
        assert!(!is_disk_full_error(raw), "shared predicate: {raw}");
        assert_ne!(
            expected_error_kind(raw),
            Some(ExpectedErrorKind::DiskFull),
            "classifier: {raw}"
        );
        let lower = raw.to_ascii_lowercase();
        for windows in [true, false] {
            assert!(!is_disk_full_os_code(&lower, windows), "os code: {raw}");
        }
    }
    // The same local text without an envelope still classifies.
    assert!(is_disk_full_error(
        "transcript persistence failed: No space left on device (os error 28)"
    ));
}
