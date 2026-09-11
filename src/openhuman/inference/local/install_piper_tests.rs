use super::*;
use crate::openhuman::inference::local::voice_install_common::reset_status;

/// Point [`paths::shared_root_dir`] at a test's own `TempDir`.
///
/// `shared_root_dir` only honours `config.workspace_dir` when
/// `OPENHUMAN_WORKSPACE` is set; without it every write below lands in the
/// developer's real `~/.openhuman/bin/piper` and the cleanup deletes their
/// installed Piper (CodeRabbit, #5253). Setting the variable for the
/// duration keeps writes *and* cleanup inside the `TempDir`, so the
/// `TempDir`'s own `Drop` is the cleanup and it runs on unwind too: a
/// failing assertion can no longer leave a stub binary behind for the next
/// test to trip over.
///
/// Callers must already hold [`shared_install_lock`] — this mutates
/// process-wide environment state.
///
/// `#[cfg(unix)]` because its only consumer is the unix-only permissions
/// test; unconditional would be dead code on Windows, where clippy runs
/// with `-D warnings`.
#[cfg(unix)]
struct SharedRootOverride {
    previous: Option<std::ffi::OsString>,
}

#[cfg(unix)]
impl SharedRootOverride {
    fn set(root: &std::path::Path) -> Self {
        let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
        std::env::set_var("OPENHUMAN_WORKSPACE", root);
        Self { previous }
    }
}

#[cfg(unix)]
impl Drop for SharedRootOverride {
    fn drop(&mut self) {
        match self.previous.as_ref() {
            Some(previous) => std::env::set_var("OPENHUMAN_WORKSPACE", previous),
            None => std::env::remove_var("OPENHUMAN_WORKSPACE"),
        }
    }
}

#[cfg(unix)]
#[test]
fn non_executable_workspace_binary_is_skipped_so_path_can_win() {
    // #5045 review (Codex P2): when the chmod repair fails, returning the
    // 0644 workspace copy anyway pins resolution to a binary that cannot
    // launch and makes the PIPER_BIN/PATH fallback unreachable.
    //
    // This test must hold the module lock: it mutates OPENHUMAN_WORKSPACE,
    // which is process-wide, and `reset_status`/install state is shared
    // with every sibling install_piper / paths test.
    //
    // `workspace_piper_binary_candidates` resolves through
    // `paths::shared_root_dir`, which ignores `config.workspace_dir` unless
    // OPENHUMAN_WORKSPACE is set and otherwise returns the real
    // `~/.openhuman/bin/piper`. `SharedRootOverride` sets it to this test's
    // TempDir so the stub written below, and its cleanup, stay inside the
    // TempDir instead of touching a developer's installed Piper.
    use std::os::unix::fs::PermissionsExt;
    let _g = shared_install_lock();
    let (_dir, config) = temp_config();
    let _root = SharedRootOverride::set(&config.workspace_dir);
    wipe_shared_install_dir(&config);
    let candidates = paths::workspace_piper_binary_candidates(&config);
    let candidate = candidates.first().expect("at least one candidate").clone();
    std::fs::create_dir_all(candidate.parent().unwrap()).unwrap();
    std::fs::write(&candidate, b"#!/bin/sh\n").unwrap();

    std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        find_workspace_piper_binary(&config).is_none(),
        "a non-executable workspace binary must not be resolved"
    );

    std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        find_workspace_piper_binary(&config).as_deref(),
        Some(candidate.as_path()),
        "an executable workspace binary is still preferred"
    );

    // No tail cleanup: it would only run when every assertion above passed.
    // `_dir` (TempDir) and `_root` (SharedRootOverride) clean up on drop,
    // which happens on the panic path too.
}

fn temp_config() -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = Config {
        workspace_dir: dir.path().join("workspace"),
        config_path: dir.path().join("config.toml"),
        ..Config::default()
    };
    (dir, config)
}

#[test]
fn decode_voice_id_splits_correctly() {
    assert_eq!(
        decode_voice_id("en_US-lessac-medium"),
        (
            "en".to_string(),
            "en_US".to_string(),
            "lessac".to_string(),
            "medium".to_string()
        )
    );
    assert_eq!(
        decode_voice_id("de_DE-thorsten-high"),
        (
            "de".to_string(),
            "de_DE".to_string(),
            "thorsten".to_string(),
            "high".to_string()
        )
    );
}

#[test]
fn decode_voice_id_falls_back_for_garbage() {
    // Single-piece input is malformed → bundled default decomposition.
    let (lang, locale, name, quality) = decode_voice_id("garbage");
    assert_eq!(lang, "en");
    assert_eq!(locale, "en_US");
    assert_eq!(name, "lessac");
    assert_eq!(quality, "medium");

    let (_lang, _locale, _name, _quality) = decode_voice_id("");
    // Empty string also produces the bundled default — guarded above.
}

#[test]
fn voice_download_urls_anchor_on_hf_bucket() {
    let (onnx, json) = voice_download_urls("en_US-lessac-medium");
    assert!(onnx.starts_with("https://huggingface.co/rhasspy/piper-voices/resolve/main/"));
    assert!(onnx.ends_with("en_US-lessac-medium.onnx"));
    assert!(json.ends_with("en_US-lessac-medium.onnx.json"));
}

#[test]
fn binary_download_asset_picks_an_os_specific_url() {
    let asset = binary_download_asset();
    // On supported platforms we expect an asset; the test only runs
    // on the host so this is informative.
    if cfg!(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "linux"
    )) {
        let asset = asset.expect("supported platform should return an asset");
        assert!(asset.url.contains("piper"));
        assert!(asset
            .url
            .starts_with("https://github.com/rhasspy/piper/releases"));
        if cfg!(windows) {
            assert_eq!(asset.kind, ArchiveKind::Zip);
        } else {
            assert_eq!(asset.kind, ArchiveKind::TarGz);
        }
    } else {
        assert!(asset.is_none());
    }
}

/// Serialise tests that write into the shared `~/.openhuman/bin/piper/`
/// directory; reuses the module-wide `local_ai_test_guard` so paths +
/// sibling installer tests are serialised through the same lock.
fn shared_install_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::openhuman::inference::inference_test_guard()
}

fn wipe_shared_install_dir(config: &Config) {
    let dir = paths::workspace_piper_dir(config);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn status_reports_missing_for_fresh_workspace() {
    let _g = shared_install_lock();
    reset_status(ENGINE_PIPER);
    let (_tmp, config) = temp_config();
    wipe_shared_install_dir(&config);
    let snapshot = status(&config);
    assert_eq!(snapshot.state, VoiceInstallState::Missing);
}

/// Build a `.onnx.json` payload big enough to pass the size floor.
/// Real Piper sidecars are a few KB; the floor exists to reject 404
/// HTML pages, so as long as we write past 256 bytes we mirror the
/// production validator's accept set.
fn synthetic_voice_json() -> Vec<u8> {
    let mut body = br#"{"audio":{"sample_rate":22050},"phoneme_id_map":{},"#.to_vec();
    // Pad to comfortably exceed the size floor without altering shape.
    body.extend_from_slice(br#""filler":""#);
    body.extend(std::iter::repeat_n(b'x', 512));
    body.extend_from_slice(br#""}"#);
    body
}

#[test]
fn status_promotes_to_installed_when_voice_and_binary_present() {
    let _g = shared_install_lock();
    reset_status(ENGINE_PIPER);
    let (_tmp, config) = temp_config();
    wipe_shared_install_dir(&config);
    // Voice files.
    let (onnx, json) =
        paths::workspace_piper_voice_paths(&config, DEFAULT_PIPER_VOICE).expect("voice paths");
    std::fs::create_dir_all(onnx.parent().unwrap()).unwrap();
    std::fs::write(&onnx, vec![0u8; (MIN_VOICE_BYTES + 1024) as usize]).unwrap();
    std::fs::write(&json, synthetic_voice_json()).unwrap();
    // Binary.
    let bin_candidate = paths::workspace_piper_binary_candidates(&config)[0].clone();
    std::fs::create_dir_all(bin_candidate.parent().unwrap()).unwrap();
    std::fs::write(&bin_candidate, b"stub").unwrap();

    let snapshot = status(&config);
    assert_eq!(snapshot.state, VoiceInstallState::Installed);
    wipe_shared_install_dir(&config);
}

// Holding the sync mutex over
// the install await is safe because the install path doesn't acquire
// any other locks, and the guard's job is to keep filesystem writes
// from racing with sibling tests.
#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn install_short_circuits_when_already_installed() {
    let _g = shared_install_lock();
    reset_status(ENGINE_PIPER);
    let (_tmp, config) = temp_config();
    wipe_shared_install_dir(&config);
    let (onnx, json) =
        paths::workspace_piper_voice_paths(&config, DEFAULT_PIPER_VOICE).expect("voice paths");
    std::fs::create_dir_all(onnx.parent().unwrap()).unwrap();
    std::fs::write(&onnx, vec![0u8; (MIN_VOICE_BYTES + 1024) as usize]).unwrap();
    std::fs::write(&json, synthetic_voice_json()).unwrap();
    let bin_candidate = paths::workspace_piper_binary_candidates(&config)[0].clone();
    std::fs::create_dir_all(bin_candidate.parent().unwrap()).unwrap();
    std::fs::write(&bin_candidate, b"stub").unwrap();

    let result = install_piper(&config, None, false).await;
    assert!(result.is_ok(), "short-circuit must succeed: {result:?}");
    let snap = result.unwrap();
    assert_eq!(snap.state, VoiceInstallState::Installed);
    wipe_shared_install_dir(&config);
}

#[test]
fn find_workspace_piper_binary_returns_path_when_present() {
    let _g = shared_install_lock();
    let (_tmp, config) = temp_config();
    wipe_shared_install_dir(&config);
    let target = paths::workspace_piper_binary_candidates(&config)[0].clone();
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, b"stub").unwrap();
    // "Present" now means present AND executable: a 0644 file is not a
    // usable binary, and resolving it would pin us to a copy that cannot
    // launch (see `non_executable_workspace_binary_is_skipped_so_path_can_win`).
    // `std::fs::write` creates 0644, so grant the bit explicitly.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let found = find_workspace_piper_binary(&config).expect("should find binary");
    assert_eq!(found, target);
    wipe_shared_install_dir(&config);
}

#[test]
fn find_workspace_piper_binary_returns_none_without_install() {
    let _g = shared_install_lock();
    let (_tmp, config) = temp_config();
    wipe_shared_install_dir(&config);
    assert!(find_workspace_piper_binary(&config).is_none());
}

/// Regression tests for #5045: the upstream macOS tarballs ship
/// `espeak-ng` as mode 0644 and `tar::Archive::unpack` reproduces the
/// archived mode verbatim, leaving Piper unable to phonemize.
#[cfg(unix)]
mod executable_bits {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode_of(path: &std::path::Path) -> u32 {
        std::fs::metadata(path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777
    }

    fn write_with_mode(path: &std::path::Path, mode: u32) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, b"stub").expect("write");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
    }

    /// The headline bug: a 0644 `espeak-ng` in the nested `piper/`
    /// layout the macOS tarball actually produces.
    #[test]
    fn repairs_non_executable_espeak_ng_in_nested_layout() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let espeak = tmp.path().join("piper").join("espeak-ng");
        write_with_mode(&espeak, 0o644);
        assert_eq!(mode_of(&espeak), 0o644, "precondition: not executable");

        ensure_executable_bits(tmp.path());

        assert_eq!(
            mode_of(&espeak),
            0o755,
            "espeak-ng must be executable after extraction"
        );
    }

    /// Some builds flatten the archive to the install root rather than
    /// nesting under `piper/`; both layouts must be repaired.
    #[test]
    fn repairs_binaries_in_flat_layout() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let espeak = tmp.path().join("espeak-ng");
        write_with_mode(&espeak, 0o644);

        ensure_executable_bits(tmp.path());

        assert_eq!(mode_of(&espeak), 0o755);
    }

    /// `piper` and `piper_phonemize` already ship 0755 upstream — the
    /// repair must not widen or otherwise rewrite a good mode.
    #[test]
    fn leaves_already_executable_binaries_untouched() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let piper = tmp.path().join("piper").join("piper");
        // Deliberately narrower than 0755 to prove we don't rewrite.
        write_with_mode(&piper, 0o700);

        ensure_executable_bits(tmp.path());

        assert_eq!(
            mode_of(&piper),
            0o700,
            "an already-executable binary must keep its mode"
        );
    }

    /// Data files shipped alongside the binaries (`libtashkeel_model.ort`,
    /// the `.onnx` voices) must not become executable.
    #[test]
    fn does_not_touch_non_binary_payloads() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let data = tmp.path().join("piper").join("libtashkeel_model.ort");
        write_with_mode(&data, 0o644);

        ensure_executable_bits(tmp.path());

        assert_eq!(
            mode_of(&data),
            0o644,
            "data payloads must not gain the execute bit"
        );
    }

    /// A missing install directory is the common case on a fresh
    /// workspace — the repair must be a silent no-op, not a panic.
    #[test]
    fn is_a_no_op_when_nothing_was_extracted() {
        let tmp = tempfile::tempdir().expect("tempdir");
        ensure_executable_bits(&tmp.path().join("does-not-exist"));
    }
}
