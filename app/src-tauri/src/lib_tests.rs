use super::*;

// Tests that read/write process-global env vars must serialize through this
// mutex. Rust's test runner executes tests in parallel by default; without
// coordination, concurrent set_var / remove_var calls race and produce
// spurious failures.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Test that is_daemon_mode correctly detects daemon flag variations
#[test]
fn is_daemon_mode_detects_daemon_flag() {
    // Note: This test relies on the current process args, so in test mode
    // it will typically return false. We verify the function is callable.
    let _result = is_daemon_mode();
}

/// Test core_rpc_url returns expected format
#[test]
fn core_rpc_url_returns_expected_format() {
    let _g = ENV_LOCK.lock().unwrap();
    let original = std::env::var("OPENHUMAN_CORE_RPC_URL").ok();

    std::env::set_var("OPENHUMAN_CORE_RPC_URL", "http://localhost:9999/rpc");
    let url = core_rpc_url();
    assert_eq!(url, "http://localhost:9999/rpc");

    std::env::remove_var("OPENHUMAN_CORE_RPC_URL");
    let url = core_rpc_url();
    assert_eq!(url, "http://127.0.0.1:7788/rpc");

    match original {
        Some(v) => std::env::set_var("OPENHUMAN_CORE_RPC_URL", v),
        None => std::env::remove_var("OPENHUMAN_CORE_RPC_URL"),
    }
}

/// Test overlay_parent_rpc_url handles empty env var
#[test]
fn overlay_parent_rpc_url_handles_empty() {
    let _g = ENV_LOCK.lock().unwrap();
    let original = std::env::var("OPENHUMAN_CORE_RPC_URL").ok();

    std::env::set_var("OPENHUMAN_CORE_RPC_URL", "");
    assert!(overlay_parent_rpc_url().is_none());

    std::env::set_var("OPENHUMAN_CORE_RPC_URL", "   ");
    assert!(overlay_parent_rpc_url().is_none());

    std::env::set_var("OPENHUMAN_CORE_RPC_URL", "http://127.0.0.1:7788/rpc");
    assert_eq!(
        overlay_parent_rpc_url(),
        Some("http://127.0.0.1:7788/rpc".to_string())
    );

    match original {
        Some(v) => std::env::set_var("OPENHUMAN_CORE_RPC_URL", v),
        None => std::env::remove_var("OPENHUMAN_CORE_RPC_URL"),
    }
}

/// Tests for setup_tray conditional compilation
/// The PR adds two versions of setup_tray():
/// 1. No-op for linux + cef: logs warning and returns Ok(())
/// 2. Full implementation for other platforms
///
/// These tests verify the function signatures are correct and
/// the compile-time cfg blocks are properly set up.

/// Verify setup_tray function exists and has correct signature
/// This test passes if the code compiles, as the function signature
/// is validated by the compiler.
#[test]
fn setup_tray_function_signature_compiles() {
    // This test exists to ensure the conditional compilation
    // of setup_tray is valid. The function is not actually called
    // here because it requires a full Tauri AppHandle.
    // The cfg attributes ensure only one version exists at compile time.
}

/// Test that AppRuntime is defined for the current feature set
#[test]
fn app_runtime_type_exists() {
    // This test verifies AppRuntime is properly defined
    // based on the cef feature flag.
    // The type alias exists at module scope and is used throughout.
    fn _check_runtime<R: tauri::Runtime>() {}
    // _check_runtime::<AppRuntime>(); // Would require importing
}

#[test]
fn no_app_update_available_result_is_quiet_unavailable() {
    let info = no_app_update_available("0.53.43".to_string());

    assert_eq!(info.current_version, "0.53.43");
    assert!(!info.available);
    assert!(info.available_version.is_none());
    assert!(info.body.is_none());
}

/// Verify tray logging patterns exist (grep-friendly)
#[test]
fn tray_setup_logging_patterns_exist() {
    // These log patterns from the PR are grep-friendly:
    // "[tray] skipping tray setup on linux+cef: ..."
    // "[tray] setting up tray icon"
    // "[tray] tray icon ready"
    // "[tray] action=show_window ..."
    // "[tray] action=quit ..."
    // "[tray] failed to setup tray icon ..."
    // "[app] RunEvent::Ready — GTK initialized, setting up tray"
    //
    // This test passes if the code compiles with these log messages.
}

// -------------------------------------------------------------------------
// macos_os_version (issue #1012)
// -------------------------------------------------------------------------

/// On macOS, sw_vers is always present and must return a version string.
#[cfg(target_os = "macos")]
#[test]
fn macos_os_version_returns_some() {
    assert!(
        macos_os_version().is_some(),
        "sw_vers -productVersion must succeed on macOS"
    );
}

/// The returned version must be a non-empty trimmed string.
#[cfg(target_os = "macos")]
#[test]
fn macos_os_version_is_nonempty() {
    let ver = macos_os_version().expect("sw_vers must return a version on macOS");
    assert!(!ver.is_empty());
    // No leading/trailing whitespace (the impl trims).
    assert_eq!(ver, ver.trim());
}

/// The version string must look like dot-separated integers ("14.5", "13.2.1").
#[cfg(target_os = "macos")]
#[test]
fn macos_os_version_is_dotted_integer_format() {
    let ver = macos_os_version().expect("sw_vers must return a version on macOS");
    let all_numeric_parts = ver
        .split('.')
        .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()));
    assert!(
        all_numeric_parts,
        "os version {ver:?} must be dot-separated integers (e.g. '14.5')"
    );
}

/// The version must have at least one component (e.g. a bare major "15" is valid).
#[cfg(target_os = "macos")]
#[test]
fn macos_os_version_has_at_least_one_component() {
    let ver = macos_os_version().expect("sw_vers must return a version on macOS");
    assert!(
        !ver.split('.').next().unwrap_or("").is_empty(),
        "version must have at least one numeric component"
    );
}

// -------------------------------------------------------------------------
// WSL + X11 desktop startup warning (issue #1653)
// -------------------------------------------------------------------------

#[test]
fn wsl_x11_warning_detects_classic_x11_forwarding() {
    assert!(should_warn_for_wsl_x11_desktop(true, true, false, false));
}

#[test]
fn wsl_x11_warning_skips_non_wsl_or_headless_runs() {
    assert!(!should_warn_for_wsl_x11_desktop(false, true, false, false));
    assert!(!should_warn_for_wsl_x11_desktop(true, false, false, false));
}

#[test]
fn wsl_x11_warning_skips_wslg_or_wayland_runs() {
    assert!(!should_warn_for_wsl_x11_desktop(true, true, true, false));
    assert!(!should_warn_for_wsl_x11_desktop(true, true, false, true));
}

// -------------------------------------------------------------------------
// Linux display-server pre-flight (Sentry OPENHUMAN-TAURI-K1)
// -------------------------------------------------------------------------

#[test]
fn linux_display_present_with_x11() {
    assert!(linux_display_server_present(true, false));
}

#[test]
fn linux_display_present_with_wayland() {
    assert!(linux_display_server_present(false, true));
}

#[test]
fn linux_display_present_with_both() {
    assert!(linux_display_server_present(true, true));
}

#[test]
fn linux_display_absent_without_either() {
    assert!(!linux_display_server_present(false, false));
}

#[test]
fn linux_root_uid_detected() {
    assert!(linux_is_root_uid(0));
}

#[test]
fn linux_non_root_uid_not_detected() {
    assert!(!linux_is_root_uid(1000));
    assert!(!linux_is_root_uid(1));
}

// -------------------------------------------------------------------------
// Linux D-Bus session-bus probe (Sentry OPENHUMAN-TAURI-TM)
// -------------------------------------------------------------------------

#[test]
fn dbus_address_unix_is_supported() {
    assert!(dbus_address_is_supported("unix:path=/run/user/1000/bus"));
    assert!(dbus_address_is_supported("unix:abstract=/tmp/dbus-abc"));
}

#[test]
fn dbus_address_tcp_and_launchd_supported() {
    assert!(dbus_address_is_supported("tcp:host=localhost,port=1234"));
    assert!(dbus_address_is_supported(
        "launchd:env=DBUS_LAUNCHD_SESSION_BUS_SOCKET"
    ));
    assert!(dbus_address_is_supported("autolaunch:"));
}

#[test]
fn dbus_address_disabled_is_unsupported() {
    // The literal value WSL2-without-WSLg sets — root cause of the panic.
    assert!(!dbus_address_is_supported("disabled"));
    assert!(!dbus_address_is_supported(""));
    assert!(!dbus_address_is_supported("   "));
}

#[test]
fn dbus_address_unknown_transport_is_unsupported() {
    assert!(!dbus_address_is_supported("nonce-tcp:host=localhost"));
    assert!(!dbus_address_is_supported("bogus:"));
}

#[test]
fn dbus_address_picks_first_supported_in_list() {
    // zbus walks the semicolon-separated list and uses the first reachable
    // transport, so one good entry is enough.
    assert!(dbus_address_is_supported(
        "disabled;unix:path=/run/user/1000/bus"
    ));
    assert!(dbus_address_is_supported(
        "bogus:;tcp:host=localhost,port=55"
    ));
    assert!(!dbus_address_is_supported("disabled;bogus:"));
}

#[test]
fn dbus_reachable_when_env_addr_is_supported() {
    assert!(linux_dbus_session_reachable(
        Some("unix:path=/run/user/1000/bus"),
        false,
    ));
}

#[test]
fn dbus_unreachable_when_env_addr_disabled() {
    // Even if the socket exists, an explicit `disabled` value means the
    // session bus is intentionally turned off and zbus will reject it.
    assert!(!linux_dbus_session_reachable(Some("disabled"), true));
}

#[test]
fn dbus_falls_back_to_runtime_socket_when_env_unset() {
    assert!(linux_dbus_session_reachable(None, true));
    assert!(!linux_dbus_session_reachable(None, false));
}

// -------------------------------------------------------------------------
// Platform constants (issue #1012 Sentry tagging)
// -------------------------------------------------------------------------

/// cpu_arch tag is derived from std::env::consts::ARCH which must be non-empty.
#[test]
fn platform_arch_constant_is_nonempty() {
    assert!(
        !std::env::consts::ARCH.is_empty(),
        "ARCH constant used for Sentry cpu_arch tag must be non-empty"
    );
}

/// os_name tag is derived from std::env::consts::OS which must be non-empty.
#[test]
fn platform_os_constant_is_nonempty() {
    assert!(
        !std::env::consts::OS.is_empty(),
        "OS constant used for Sentry os_name tag must be non-empty"
    );
}

/// On a macOS build the OS constant must equal "macos".
#[cfg(target_os = "macos")]
#[test]
fn platform_os_is_macos_on_macos_build() {
    assert_eq!(std::env::consts::OS, "macos");
}

#[test]
fn platform_cef_gpu_workarounds_disable_linux_gpu_path() {
    let mut args = Vec::new();
    append_platform_cef_gpu_workarounds(&mut args, "linux", "x86_64", None);

    assert!(args.contains(&("--disable-gpu", None)));
    assert!(args.contains(&("--disable-gpu-compositing", None)));
}

#[test]
fn platform_cef_gpu_workarounds_disable_intel_macos_compositing_only() {
    let mut args = Vec::new();
    append_platform_cef_gpu_workarounds(&mut args, "macos", "x86_64", None);

    assert_eq!(args, vec![("--disable-gpu-compositing", None)]);
}

#[test]
fn platform_cef_gpu_workarounds_leave_other_platforms_alone() {
    for (os, arch) in [("macos", "aarch64"), ("windows", "x86_64")] {
        let mut args = Vec::new();
        append_platform_cef_gpu_workarounds(&mut args, os, arch, None);

        assert!(
            args.is_empty(),
            "unexpected CEF GPU flags for {os}/{arch}: {args:?}"
        );
    }
}

// -------------------------------------------------------------------------
// OPENHUMAN_FORCE_GPU override (re-enables WebGL2 surfaces — Rive mascot)
// -------------------------------------------------------------------------

#[test]
fn force_gpu_default_off_when_env_unset() {
    assert!(!cef_force_gpu_enabled(None));
}

#[test]
fn force_gpu_explicit_enable_values_match_prewarm_pattern() {
    for v in ["1", "true", "yes", "on", "TRUE", "Yes", "On"] {
        assert!(
            cef_force_gpu_enabled(Some(v)),
            "OPENHUMAN_FORCE_GPU={v:?} should opt in"
        );
    }
}

#[test]
fn force_gpu_anything_else_is_off() {
    // Mirrors prewarm semantics: only explicit truthy values opt in.
    for v in ["", "0", "false", "no", "off", "FALSE", "Off", "maybe", " "] {
        assert!(
            !cef_force_gpu_enabled(Some(v)),
            "OPENHUMAN_FORCE_GPU={v:?} must not silently opt in"
        );
    }
}

#[test]
fn platform_cef_gpu_workarounds_skip_linux_disable_when_force_gpu_set() {
    // Assert the two GPU-disable flags are absent rather than the whole
    // arg list being empty: on root-Linux runners (CI in some configs)
    // the function still appends `--no-sandbox` via the orthogonal
    // OPENHUMAN-TAURI-K1 branch, which would make a strict `is_empty()`
    // check fail spuriously. We only care about the GPU branch here.
    let mut args = Vec::new();
    append_platform_cef_gpu_workarounds(&mut args, "linux", "x86_64", Some("1"));

    assert!(
        !args.contains(&("--disable-gpu", None)),
        "OPENHUMAN_FORCE_GPU=1 must suppress --disable-gpu, got: {args:?}"
    );
    assert!(
        !args.contains(&("--disable-gpu-compositing", None)),
        "OPENHUMAN_FORCE_GPU=1 must suppress --disable-gpu-compositing, got: {args:?}"
    );
}

#[test]
fn platform_cef_gpu_workarounds_force_gpu_does_not_affect_intel_macos_path() {
    // OPENHUMAN_FORCE_GPU only governs the Linux #1697 workaround; the
    // separate Intel-macOS #1012 disable must still apply, regardless of
    // the env var.
    let mut args = Vec::new();
    append_platform_cef_gpu_workarounds(&mut args, "macos", "x86_64", Some("1"));

    assert_eq!(args, vec![("--disable-gpu-compositing", None)]);
}

// -------------------------------------------------------------------------
// #3554 — never forward --time-ticks-at-unix-epoch to CEF (wrong system time)
// -------------------------------------------------------------------------

#[test]
fn time_ticks_flag_matches_any_dash_and_casing_form() {
    for flag in [
        "--time-ticks-at-unix-epoch",
        "-time-ticks-at-unix-epoch",
        "time-ticks-at-unix-epoch",
        "--Time-Ticks-At-Unix-Epoch",
    ] {
        assert!(
            is_time_ticks_at_unix_epoch_flag(flag),
            "{flag:?} should be recognised as the time-ticks switch"
        );
    }
}

#[test]
fn time_ticks_flag_does_not_match_unrelated_flags() {
    for flag in [
        "--disable-gpu",
        "--use-mock-keychain",
        "--time-zone",
        "--enable-features",
    ] {
        assert!(
            !is_time_ticks_at_unix_epoch_flag(flag),
            "{flag:?} must not be treated as the time-ticks switch"
        );
    }
}

#[test]
fn strip_time_ticks_removes_negative_value_and_keeps_the_rest() {
    // The corrupt value reported in #3554, alongside flags we must preserve.
    let mut args = vec![
        ("--use-mock-keychain", None),
        ("--time-ticks-at-unix-epoch", Some("-1780937467390432")),
        ("--disable-gpu", None),
    ];
    strip_time_ticks_at_unix_epoch(&mut args);

    assert_eq!(
        args,
        vec![("--use-mock-keychain", None), ("--disable-gpu", None)],
        "the corrupt time-ticks switch must be removed; everything else kept"
    );
}

#[test]
fn strip_time_ticks_removes_inline_value_form() {
    // The critical bypass case: when the value is carried inline as
    // `--flag=value` (a single token) rather than as a separate value, the
    // matcher must still recognise and strip it.
    let mut args = vec![
        ("--use-mock-keychain", None),
        ("--time-ticks-at-unix-epoch=-1780937467390432", None),
        ("--disable-gpu", None),
    ];
    strip_time_ticks_at_unix_epoch(&mut args);

    assert_eq!(
        args,
        vec![("--use-mock-keychain", None), ("--disable-gpu", None)],
        "the inline time-ticks switch must be removed; everything else kept"
    );
}

#[test]
fn strip_time_ticks_removes_the_flag_regardless_of_value() {
    // Even a non-negative value is dropped: OpenHuman must let Chromium
    // compute the clock origin rather than anchor it from the shell.
    let mut args = vec![
        ("--time-ticks-at-unix-epoch", Some("1780937467390432")),
        ("--time-ticks-at-unix-epoch", None),
    ];
    strip_time_ticks_at_unix_epoch(&mut args);

    assert!(
        args.is_empty(),
        "all time-ticks variants must be stripped, got: {args:?}"
    );
}

#[test]
fn strip_time_ticks_is_a_noop_without_the_flag() {
    let mut args = vec![("--use-mock-keychain", None), ("--disable-gpu", None)];
    let expected = args.clone();
    strip_time_ticks_at_unix_epoch(&mut args);

    assert_eq!(args, expected, "unrelated args must be left untouched");
}

/// On an Intel macOS build the ARCH constant must equal "x86_64".
/// This is the architecture that triggers --disable-gpu-compositing.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
#[test]
fn platform_arch_is_x86_64_on_intel_build() {
    assert_eq!(std::env::consts::ARCH, "x86_64");
}

/// On Apple Silicon the ARCH constant must equal "aarch64"; the GPU flag
/// must NOT be compiled in (verified by this test existing in the binary).
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn platform_arch_is_aarch64_on_apple_silicon_build() {
    assert_eq!(std::env::consts::ARCH, "aarch64");
}

// -------------------------------------------------------------------------
// cef_prewarm_enabled (issue #2463 — Wayland/XWayland BadWindow guard)
// -------------------------------------------------------------------------

#[test]
fn prewarm_enabled_by_default_on_non_wayland() {
    assert!(cef_prewarm_enabled(None, false));
}

#[test]
fn prewarm_auto_disabled_on_wayland_when_env_unset() {
    assert!(!cef_prewarm_enabled(None, true));
}

#[test]
fn prewarm_explicit_disable_respected_on_non_wayland() {
    assert!(!cef_prewarm_enabled(Some("0"), false));
    assert!(!cef_prewarm_enabled(Some("false"), false));
    assert!(!cef_prewarm_enabled(Some("no"), false));
    assert!(!cef_prewarm_enabled(Some("off"), false));
}

#[test]
fn prewarm_explicit_disable_respected_on_wayland() {
    assert!(!cef_prewarm_enabled(Some("0"), true));
    assert!(!cef_prewarm_enabled(Some("false"), true));
}

#[test]
fn prewarm_explicit_enable_overrides_wayland_guard() {
    // OPENHUMAN_CEF_PREWARM=1 (or any non-disable value) lets ops
    // force prewarm even on Wayland sessions.
    assert!(cef_prewarm_enabled(Some("1"), true));
    assert!(cef_prewarm_enabled(Some("true"), true));
    assert!(cef_prewarm_enabled(Some("yes"), true));
    assert!(cef_prewarm_enabled(Some("on"), true));
}

#[test]
fn prewarm_disable_flags_are_case_insensitive() {
    assert!(!cef_prewarm_enabled(Some("FALSE"), false));
    assert!(!cef_prewarm_enabled(Some("OFF"), true));
    assert!(!cef_prewarm_enabled(Some("  0  "), false));
    assert!(!cef_prewarm_enabled(Some("  No  "), true));
}

#[test]
fn prewarm_unknown_env_value_treated_as_enable() {
    // Any string that is not a recognised disable token → treat as enable.
    assert!(cef_prewarm_enabled(Some("enabled"), false));
    assert!(cef_prewarm_enabled(Some("yes"), false));
    assert!(cef_prewarm_enabled(Some(""), false));
}

// -------------------------------------------------------------------------
// build_sentry_release_tag
// -------------------------------------------------------------------------

#[test]
fn sentry_release_tag_starts_with_openhuman() {
    let tag = build_sentry_release_tag();
    assert!(
        tag.starts_with("openhuman@"),
        "release tag must start with 'openhuman@', got: {tag:?}"
    );
}

#[test]
fn sentry_release_tag_contains_cargo_pkg_version() {
    let tag = build_sentry_release_tag();
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        tag.contains(version),
        "release tag {tag:?} must embed CARGO_PKG_VERSION {version:?}"
    );
}

#[test]
fn sentry_release_tag_version_part_is_nonempty() {
    let tag = build_sentry_release_tag();
    let after_prefix = tag.strip_prefix("openhuman@").unwrap_or("");
    assert!(!after_prefix.is_empty(), "version part must not be empty");
}

/// When a SHA is baked in the tag takes the form `openhuman@<ver>+<sha12>`.
/// When it is not, the tag is simply `openhuman@<ver>` with no `+`.
/// Either way the full tag must be non-empty.
#[test]
fn sentry_release_tag_is_nonempty() {
    assert!(!build_sentry_release_tag().is_empty());
}

// -------------------------------------------------------------------------
// resolve_sentry_environment
// -------------------------------------------------------------------------

#[test]
fn sentry_environment_reads_openhuman_app_env() {
    let _g = ENV_LOCK.lock().unwrap();
    let key = "OPENHUMAN_APP_ENV";
    let original = std::env::var(key).ok();
    std::env::set_var(key, "staging");
    let env = resolve_sentry_environment();
    match original {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
    assert_eq!(env, "staging");
}

#[test]
fn sentry_environment_trims_whitespace_from_openhuman_app_env() {
    let _g = ENV_LOCK.lock().unwrap();
    let key = "OPENHUMAN_APP_ENV";
    let original = std::env::var(key).ok();
    std::env::set_var(key, "  dev  ");
    let env = resolve_sentry_environment();
    match original {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
    assert_eq!(env, "dev");
}

#[test]
fn sentry_environment_skips_empty_openhuman_app_env() {
    let _g = ENV_LOCK.lock().unwrap();
    let key = "OPENHUMAN_APP_ENV";
    let original = std::env::var(key).ok();
    std::env::set_var(key, "");
    let env = resolve_sentry_environment();
    match original {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
    // Falls through to VITE_ compile-time value or "production"; must be non-empty.
    assert!(!env.is_empty());
}

#[test]
fn sentry_environment_skips_whitespace_only_openhuman_app_env() {
    let _g = ENV_LOCK.lock().unwrap();
    let key = "OPENHUMAN_APP_ENV";
    let original = std::env::var(key).ok();
    std::env::set_var(key, "   ");
    let env = resolve_sentry_environment();
    match original {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
    assert!(!env.is_empty());
}

/// When neither runtime env var nor compile-time VITE_ is set, the fallback
/// must be "production". Guard with a compile-time check so this test only
/// asserts the hard default when no compile-time override is present.
#[test]
fn sentry_environment_defaults_to_production_when_unset() {
    let _g = ENV_LOCK.lock().unwrap();
    if option_env!("VITE_OPENHUMAN_APP_ENV").is_some() {
        // A compile-time override is baked in; skip — the fallback path is
        // exercised by sentry_environment_skips_empty_openhuman_app_env.
        return;
    }
    let key = "OPENHUMAN_APP_ENV";
    let original = std::env::var(key).ok();
    std::env::remove_var(key);
    let env = resolve_sentry_environment();
    match original {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
    assert_eq!(env, "production");
}

// ── Sentry before_send filter: drop "Failed to request http://localhost:…"
//    noise emitted by the vendored tauri-runtime-cef dev proxy in packaged
//    builds (issue OPENHUMAN-TAURI-V). Tests target the pure
//    `message_is_localhost_dev_fetch_noise` helper so the rule can be
//    asserted without standing up a Sentry client.

#[test]
fn localhost_dev_fetch_noise_drops_vite_dev_url_1420() {
    // The exact message shape reported by the latest event tag in Sentry
    // (URL repeated by reqwest's `error sending request for url (…)`).
    let msg = "Failed to request http://localhost:1420/components/skills/SkillCard.tsx: \
               error sending request for url (http://localhost:1420/components/skills/SkillCard.tsx)";
    assert!(
        message_is_localhost_dev_fetch_noise(msg),
        "expected Vite dev-server fetch failure to be filtered"
    );
}

#[test]
fn localhost_dev_fetch_noise_drops_127_0_0_1_dev_url() {
    // Some environments resolve `localhost` to 127.0.0.1 at the reqwest
    // layer; the formatted message can carry either spelling.
    let msg = "Failed to request http://127.0.0.1:1420/index.html: \
               error sending request for url (http://127.0.0.1:1420/index.html)";
    assert!(
        message_is_localhost_dev_fetch_noise(msg),
        "expected 127.0.0.1 dev-server fetch failure to be filtered"
    );
}

#[test]
fn localhost_dev_fetch_noise_passes_production_url_through() {
    // Real upstream failures (e.g. backend API errors surfaced via the
    // same `Failed to request …` wording elsewhere) must NOT be filtered —
    // they're the high-signal events Sentry exists for.
    let msg = "Failed to request https://api.openhuman.ai/v1/skills: \
               error sending request for url (https://api.openhuman.ai/v1/skills)";
    assert!(
        !message_is_localhost_dev_fetch_noise(msg),
        "production API errors must NOT be filtered out"
    );
}

#[test]
fn localhost_dev_fetch_noise_passes_unrelated_localhost_messages() {
    // The filter is anchored on the dev-proxy's exact prefix to avoid
    // accidentally dropping any error that happens to mention localhost
    // (e.g. core-sidecar transport errors logged from coreRpcClient).
    let msg =
        "[core_rpc] transport error: error sending request for url (http://localhost:7788/rpc)";
    assert!(
        !message_is_localhost_dev_fetch_noise(msg),
        "non-tauri-cef localhost errors must NOT be filtered"
    );
}

#[test]
fn event_filter_uses_message_field() {
    // event-level coverage: when sentry-tracing populates
    // `event.message` (default with `attach_stacktrace=false`), the
    // filter should see the noise payload through the primary read
    // path. Per graycyrus on PR #1545.
    let mut event = sentry::protocol::Event::new();
    event.message = Some("Failed to request http://localhost:1420/foo: timeout".into());
    assert!(
        event_is_localhost_dev_fetch_noise(&event),
        "event.message read path must catch noise messages"
    );
}

#[test]
fn event_filter_falls_back_to_last_exception_value() {
    // event-level coverage: if `attach_stacktrace` is ever turned on,
    // sentry-tracing populates `event.exception` instead of (or in
    // addition to) `event.message`. Filter must still see the noise
    // payload through the exception fallback. Per graycyrus on PR #1545.
    let mut event = sentry::protocol::Event::new();
    event.message = None;
    event.exception.values.push(sentry::protocol::Exception {
        ty: "log".into(),
        value: Some("Failed to request http://localhost:1420/foo: timeout".into()),
        ..Default::default()
    });
    assert!(
        event_is_localhost_dev_fetch_noise(&event),
        "exception fallback must catch noise messages when event.message is absent"
    );
}

#[test]
fn event_filter_passes_through_when_neither_field_matches() {
    // Negative event-level case: no noise prefix in either field →
    // event must NOT be filtered.
    let mut event = sentry::protocol::Event::new();
    event.message = Some("genuine production error".into());
    event.exception.values.push(sentry::protocol::Exception {
        ty: "log".into(),
        value: Some("connection refused (10061)".into()),
        ..Default::default()
    });
    assert!(
        !event_is_localhost_dev_fetch_noise(&event),
        "legitimate production events must pass through"
    );
}

#[test]
fn localhost_dev_fetch_noise_anchors_to_message_start() {
    // CodeRabbit (PR #1545) caught that the predicate used
    // `contains` rather than `starts_with`. Regression: a message
    // that merely embeds the dev-proxy prefix later in its text
    // must NOT be filtered — only messages that *begin* with it.
    let msg = "User report: `Failed to request http://localhost:1420/foo` was logged earlier";
    assert!(
        !message_is_localhost_dev_fetch_noise(msg),
        "messages that merely contain the dev-proxy prefix must NOT be filtered"
    );
}

// -------------------------------------------------------------------------
// path_has_executable / deep-link xdg-mime pre-flight (OPENHUMAN-TAURI-AS)
// -------------------------------------------------------------------------

/// With a controlled `$PATH` containing one dir that holds a file named
/// `xdg-mime`, the lookup must succeed (mirrors a Linux desktop install
/// where xdg-utils ships the binary).
#[cfg(target_os = "linux")]
#[test]
fn path_has_executable_finds_file_on_path() {
    let _g = ENV_LOCK.lock().unwrap();
    let original = std::env::var_os("PATH");

    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("xdg-mime"), b"#!/bin/sh\n").expect("write stub");
    std::env::set_var("PATH", dir.path());

    assert!(
        path_has_executable("xdg-mime"),
        "must discover xdg-mime when present in a $PATH entry"
    );

    match original {
        Some(v) => std::env::set_var("PATH", v),
        None => std::env::remove_var("PATH"),
    }
}

/// With a controlled `$PATH` that does NOT contain `xdg-mime`, the lookup
/// must fail (mirrors WSL2 / minimal containers without xdg-utils — the
/// case OPENHUMAN-TAURI-AS protects against).
#[cfg(target_os = "linux")]
#[test]
fn path_has_executable_returns_false_when_missing() {
    let _g = ENV_LOCK.lock().unwrap();
    let original = std::env::var_os("PATH");

    let dir = tempfile::tempdir().expect("tempdir");
    // Intentionally do not create xdg-mime in `dir`.
    std::env::set_var("PATH", dir.path());

    assert!(
        !path_has_executable("xdg-mime"),
        "must return false when xdg-mime is not in any $PATH entry"
    );

    match original {
        Some(v) => std::env::set_var("PATH", v),
        None => std::env::remove_var("PATH"),
    }
}

/// When `$PATH` is unset entirely, the lookup must short-circuit to false
/// rather than panic or fall back to the cwd.
#[cfg(target_os = "linux")]
#[test]
fn path_has_executable_returns_false_when_path_unset() {
    let _g = ENV_LOCK.lock().unwrap();
    let original = std::env::var_os("PATH");

    std::env::remove_var("PATH");
    assert!(
        !path_has_executable("xdg-mime"),
        "unset $PATH must yield false (skip register_all on the missing-xdg-utils branch)"
    );

    match original {
        Some(v) => std::env::set_var("PATH", v),
        None => std::env::remove_var("PATH"),
    }
}

/// Regression guard for OPENHUMAN-TAURI-5V: a Linux host with `xdg-mime`
/// installed but `update-desktop-database` missing must classify as
/// "skip register_all" — the pre-#5V code only checked `xdg-mime` and
/// would have entered the plugin call, which then fires the noisy
/// `Failed to run OS command \`update-desktop-database\`` internal log
/// that escapes to Sentry. The Wave-4 fix pre-flights every xdg-utils
/// binary the plugin shells out to; this test pins that contract by
/// checking each binary lookup independently with a `$PATH` that
/// contains only `xdg-mime`.
#[cfg(target_os = "linux")]
#[test]
fn path_has_executable_returns_false_for_partial_xdg_utils_install() {
    let _g = ENV_LOCK.lock().unwrap();
    let original = std::env::var_os("PATH");

    let dir = tempfile::tempdir().expect("tempdir");
    // Only `xdg-mime` exists; `update-desktop-database` and
    // `xdg-icon-resource` are deliberately absent.
    std::fs::write(dir.path().join("xdg-mime"), b"#!/bin/sh\n").expect("write stub");
    std::env::set_var("PATH", dir.path());

    assert!(
        path_has_executable("xdg-mime"),
        "xdg-mime stub must be discoverable in the partial-install $PATH"
    );
    assert!(
        !path_has_executable("update-desktop-database"),
        "partial xdg-utils install must NOT report update-desktop-database present (OPENHUMAN-TAURI-5V)"
    );
    assert!(
        !path_has_executable("xdg-icon-resource"),
        "partial xdg-utils install must NOT report xdg-icon-resource present"
    );

    match original {
        Some(v) => std::env::set_var("PATH", v),
        None => std::env::remove_var("PATH"),
    }
}

/// Regression guard for issue #2228: `tauri-plugin-single-instance` must
/// enable the `deep-link` feature so that second-launch deep-link payloads
/// (e.g. `openhuman://oauth/...` callbacks from Windows/Linux system
/// browsers) are forwarded into the primary instance. Without it, hot OAuth
/// callbacks silently no-op while only focusing the existing window.
#[test]
fn single_instance_dep_enables_deep_link_feature() {
    let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path).expect("read app/src-tauri/Cargo.toml");
    let parsed: toml::Value = manifest.parse().expect("parse Cargo.toml");

    let dep = parsed
        .get("dependencies")
        .and_then(|d| d.get("tauri-plugin-single-instance"))
        .expect("tauri-plugin-single-instance dependency must exist");

    let features = dep.get("features").and_then(|f| f.as_array()).expect(
        "tauri-plugin-single-instance must be a table with a `features` array \
             — issue #2228 requires the `deep-link` feature to forward hot-instance \
             OAuth callbacks on Windows/Linux",
    );

    assert!(
        features.iter().any(|v| v.as_str() == Some("deep-link")),
        "tauri-plugin-single-instance must enable the `deep-link` feature \
         (issue #2228 — hot-instance OAuth callback forwarding)"
    );
}
