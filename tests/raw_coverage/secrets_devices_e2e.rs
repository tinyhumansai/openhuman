//! RPC-level e2e coverage for the local credential surface:
//! `openhuman.encrypt_secret` / `decrypt_secret`, `openhuman.security_policy_info`,
//! `openhuman.keyring_consent_*`, `openhuman.devices_*`, and the two uncovered
//! `openhuman.wallet_*` controllers.
//!
//! Nothing here needs a backend — every case is local state, which is what makes
//! the assertions exact rather than tolerant.
//!
//! A module of the aggregated `raw_coverage_all` target, not a target of its
//! own. Run with:
//!   cargo test --test raw_coverage_all --features "$(bash scripts/ci/product-features.sh)" \
//!       -- secrets_devices_e2e

#[path = "w4_shared/mod.rs"]
mod support;

use serde_json::{json, Value};
use support::{assert_no_error, error_message, logs, peel, Harness};

/// A `Config` pointing at the harness's workspace, for the few cases that seed
/// domain state directly rather than through an RPC that cannot create it.
fn config_for(harness: &Harness) -> openhuman_core::openhuman::config::Config {
    openhuman_core::openhuman::config::Config {
        workspace_dir: harness.workspace(),
        ..Default::default()
    }
}

// ── encrypt / decrypt ────────────────────────────────────────────────────────

/// The `encrypt_secret` → `decrypt_secret` round trip, plus the two edge shapes
/// the codec is documented to have.
#[tokio::test]
async fn encrypt_and_decrypt_secret_round_trip() {
    let _lock = support::env_lock();
    let harness = Harness::start("", true).await;
    let plaintext = "correct horse battery staple";

    let encrypted = harness
        .call(
            10,
            "openhuman.encrypt_secret",
            json!({ "plaintext": plaintext }),
        )
        .await;
    let encrypted_outer = assert_no_error(&encrypted, "encrypt_secret");
    assert!(
        logs(encrypted_outer).iter().any(|l| l == "secret encrypted"),
        "the outcome must carry its log line: {encrypted_outer}"
    );
    let ciphertext = peel(encrypted_outer)
        .as_str()
        .unwrap_or_else(|| panic!("encrypt_secret must return a string: {encrypted_outer}"))
        .to_string();

    assert!(
        ciphertext.starts_with("enc2:"),
        "the current codec is ChaCha20-Poly1305 behind an `enc2:` tag; an \
         untagged value would be handed back verbatim by `decrypt_secret` and \
         so would never be detected as unencrypted: {ciphertext}"
    );
    assert!(
        !ciphertext.contains("horse"),
        "the plaintext must not survive into the ciphertext: {ciphertext}"
    );
    assert!(
        ciphertext.len() > plaintext.len(),
        "nonce + tag must be present: {ciphertext}"
    );

    let decrypted = harness
        .call(
            11,
            "openhuman.decrypt_secret",
            json!({ "ciphertext": ciphertext }),
        )
        .await;
    let decrypted_outer = assert_no_error(&decrypted, "decrypt_secret");
    assert_eq!(
        peel(decrypted_outer).as_str(),
        Some(plaintext),
        "the round trip must return the exact plaintext: {decrypted_outer}"
    );

    // Encrypting twice must not produce the same ciphertext — a fresh nonce per
    // call is what stops two equal secrets being linkable on disk.
    let again = harness
        .call(
            12,
            "openhuman.encrypt_secret",
            json!({ "plaintext": plaintext }),
        )
        .await;
    let second = peel(assert_no_error(&again, "encrypt_secret twice"))
        .as_str()
        .expect("encrypt_secret must return a string")
        .to_string();
    assert_ne!(
        second, ciphertext,
        "the same plaintext must encrypt to a different ciphertext each time"
    );
    let decrypted_second = harness
        .call(
            13,
            "openhuman.decrypt_secret",
            json!({ "ciphertext": second }),
        )
        .await;
    assert_eq!(
        peel(assert_no_error(&decrypted_second, "decrypt second")).as_str(),
        Some(plaintext),
        "both ciphertexts must decrypt back to the same plaintext"
    );
}

/// The failure and pass-through shapes of the codec.
#[tokio::test]
async fn decrypt_secret_rejects_a_corrupted_payload_and_passes_plaintext_through() {
    let _lock = support::env_lock();
    let harness = Harness::start("", true).await;

    let encrypted = harness
        .call(20, "openhuman.encrypt_secret", json!({ "plaintext": "secret" }))
        .await;
    let ciphertext = peel(assert_no_error(&encrypted, "encrypt_secret"))
        .as_str()
        .expect("ciphertext")
        .to_string();

    // Flip the last hex nibble — the AEAD tag must reject it rather than
    // returning garbage plaintext.
    let mut corrupted: Vec<char> = ciphertext.chars().collect();
    let last = corrupted.len() - 1;
    corrupted[last] = if corrupted[last] == '0' { '1' } else { '0' };
    let corrupted: String = corrupted.into_iter().collect();
    assert_ne!(corrupted, ciphertext, "the corruption must actually change a byte");

    let failed = harness
        .call(
            21,
            "openhuman.decrypt_secret",
            json!({ "ciphertext": corrupted }),
        )
        .await;
    error_message(&failed, "decrypt_secret corrupted payload");

    // An untagged value is documented as returned as-is (plaintext config
    // predates the codec). Pinned so the pass-through can't be removed silently.
    let untagged = harness
        .call(
            22,
            "openhuman.decrypt_secret",
            json!({ "ciphertext": "plain-config-value" }),
        )
        .await;
    assert_eq!(
        peel(assert_no_error(&untagged, "decrypt_secret untagged")).as_str(),
        Some("plain-config-value"),
        "an unprefixed value must pass through unchanged, not error"
    );

    // A missing param is a controller-level error, not a panic.
    let missing = harness.call(23, "openhuman.decrypt_secret", json!({})).await;
    assert!(
        error_message(&missing, "decrypt_secret no params").contains("ciphertext"),
        "the error must name the parameter that was missing: {missing}"
    );
}

// ── security policy ──────────────────────────────────────────────────────────

/// `security_policy_info` is a pure projection of `[autonomy]`. Non-default
/// values are used throughout so a handler that returned struct defaults, or
/// read the wrong config block, fails here.
#[tokio::test]
async fn security_policy_info_reflects_the_configured_autonomy_block() {
    let _lock = support::env_lock();
    // `schema_version` above `CURRENT_SCHEMA_VERSION` makes `run_pending` return
    // early, so no startup migration touches this fixture. Without it the 3->4
    // "expand autonomy defaults" migration merges 28 commands into
    // `allowed_commands` — see the ignored case below and
    // `~/tinyhuman/bugs/e2e-wave-autonomy-migration-widens-allowlist.md`.
    let harness = Harness::start(
        r#"schema_version = 99

[autonomy]
level = "full"
workspace_only = false
allowed_commands = ["ls", "cat", "w4-only-command"]
max_actions_per_hour = 137
require_approval_for_medium_risk = false
block_high_risk_commands = false
"#,
        true,
    )
    .await;

    let info = harness
        .call(30, "openhuman.security_policy_info", json!({}))
        .await;
    let info_outer = assert_no_error(&info, "security_policy_info");
    assert!(
        logs(info_outer)
            .iter()
            .any(|l| l.contains("computed from active config")),
        "the log line must say the payload came from live config: {info_outer}"
    );
    let info = peel(info_outer);

    assert_eq!(
        info.get("autonomy").and_then(Value::as_str),
        Some("full"),
        "the autonomy level must be the configured one, not the `supervised` \
         default: {info}"
    );
    assert_eq!(
        info.get("workspace_only").and_then(Value::as_bool),
        Some(false),
        "workspace_only defaults to true, so `false` here proves config was read: {info}"
    );
    assert_eq!(
        info.get("max_actions_per_hour").and_then(Value::as_u64),
        Some(137),
        "the rate cap must come from config: {info}"
    );
    assert_eq!(
        info.get("require_approval_for_medium_risk")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        info.get("block_high_risk_commands").and_then(Value::as_bool),
        Some(false)
    );

    let allowed: Vec<&str> = info
        .get("allowed_commands")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("the payload must carry `allowed_commands`: {info}"))
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        allowed,
        vec!["ls", "cat", "w4-only-command"],
        "the allowlist must be the configured one, in order: {info}"
    );
}

/// A narrowed `allowed_commands` must stay narrowed.
///
/// **Currently fails.** `schema_version` is `#[serde(default)]` → `0`, so a
/// `config.toml` written by hand (or by anything that omits the field) is treated
/// as pre-v4 and the 3->4 "expand autonomy defaults" migration merges 28 commands
/// into the user's allowlist — including the filesystem-mutating `mkdir`, `touch`,
/// `cp`, `mv`, `ln`. The migration's own doc claims "deliberate removals" are
/// preserved; they are not.
///
/// Ignored rather than deleted so the intended contract stays written down.
/// See `~/tinyhuman/bugs/e2e-wave-autonomy-migration-widens-allowlist.md`.
#[ignore = "openhuman: schema_version defaults to 0, so the 3->4 autonomy \
            migration widens a hand-written allowed_commands — see \
            ~/tinyhuman/bugs/e2e-wave-autonomy-migration-widens-allowlist.md"]
#[tokio::test]
async fn security_policy_info_does_not_widen_a_narrowed_command_allowlist() {
    let _lock = support::env_lock();
    // Deliberately no `schema_version`: this is what a hand-written config looks like.
    let harness = Harness::start(
        r#"
[autonomy]
allowed_commands = ["ls", "cat"]
"#,
        true,
    )
    .await;

    let info = harness
        .call(35, "openhuman.security_policy_info", json!({}))
        .await;
    let info = peel(assert_no_error(&info, "security_policy_info narrowed allowlist"));
    let allowed: Vec<&str> = info
        .get("allowed_commands")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("the payload must carry `allowed_commands`: {info}"))
        .iter()
        .filter_map(Value::as_str)
        .collect();

    assert_eq!(
        allowed,
        vec!["ls", "cat"],
        "a config that allows exactly two commands must not gain 28 more: {allowed:?}"
    );
    for widened in ["mkdir", "touch", "cp", "mv", "ln"] {
        assert!(
            !allowed.contains(&widened),
            "`{widened}` mutates the filesystem and was never allowed by this \
             config, but the startup migration added it: {allowed:?}"
        );
    }
}

// ── keyring consent ──────────────────────────────────────────────────────────

/// Assert the BACKEND -> MODE pairing, not merely that the mode is in the enum.
///
/// #6076 was exactly a mismatched pair — `backendName: "file"` with
/// `activeMode: "os_keyring"` — so a predicate that only rejects
/// `consent_pending`/`declined` would let that regression back in while a comment
/// claimed to guard it. Mirrors `active_mode_for` in
/// `security/keyring_consent/policy.rs`, including the branch #6139's inline
/// version could not reach: an `os` backend whose probe FAILED falls back to the
/// recorded consent, not to `consent_pending` unconditionally.
///
/// `label` names the call site, so a failure says which of the two surfaces broke.
fn assert_mode_matches_backend(status: &Value, label: &str) {
    let available = status
        .get("available")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| panic!("{label}: `available` must be a boolean: {status}"));
    let backend = status
        .get("backendName")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{label}: the backend must be named: {status}"));
    let mode = status
        .get("activeMode")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{label}: status must report an `activeMode`: {status}"));

    match backend {
        "os" if available => assert_eq!(
            mode, "os_keyring",
            "{label}: a working OS credential store is the one case that may claim \
             `os_keyring`: {status}"
        ),
        // Probe failed: the mode falls back to whatever consent was recorded.
        "os" => assert!(
            ["consent_pending", "local_encrypted", "declined"].contains(&mode),
            "{label}: an unavailable OS keyring reports the recorded consent: {status}"
        ),
        "encrypted_file" => assert_eq!(
            mode, "local_encrypted_file",
            "{label}: the encrypted_file backend stores secrets in \
             {{workspace}}/secrets.enc: {status}"
        ),
        // What CI runs: no OS credential store, so secrets are plaintext on disk.
        "file" | "mock" => assert_eq!(
            mode, "local_plaintext_file",
            "{label}: the file/mock backend is plaintext dev-keychain.json and must \
             say so rather than claiming the OS keyring: {status}"
        ),
        // An unrecognised backend claims nothing — the deliberate fallback in policy.rs.
        _ => assert_eq!(
            mode, "consent_pending",
            "{label}: an unrecognised backend must not guess where secrets live: {status}"
        ),
    }
}

/// All three keyring-consent controllers.
///
/// The anchor is the **persisted file**, not `activeMode`. `policy::current_status`
/// reported `os_keyring` whenever *any* backend probes as available (fixed by
/// #6096; `activeMode` now names the backend actually in use) — and the
/// `file` dev backend always does — so `activeMode` cannot witness a consent
/// decision in a test process. `ops::keyring_consent_decide` documents that the
/// in-memory cache is only updated *after* a successful persist, so proving the
/// persist landed is the assertion that actually bites. See
/// `~/tinyhuman/bugs/e2e-wave-keyring-consent-os-keyring-mislabel.md`.
#[tokio::test]
async fn keyring_consent_status_decide_and_retry_probe() {
    let _lock = support::env_lock();
    let harness = Harness::start("", true).await;
    let stored_state = harness.workspace().join("state").join("app-state.json");

    fn stored_consent(path: &std::path::Path) -> Value {
        let raw = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let state: Value = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
        state
            .get("keyringConsent")
            .cloned()
            .unwrap_or_else(|| panic!("no `keyringConsent` in {raw}"))
    }

    // --- keyring_consent_status --------------------------------------------
    let status = harness
        .call(40, "openhuman.keyring_consent_status", json!({}))
        .await;
    let status = peel(assert_no_error(&status, "keyring_consent_status"));
    let available = status
        .get("available")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| panic!("`available` must be a boolean the UI can branch on: {status}"));
    let mode = status
        .get("activeMode")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("status must report an `activeMode`: {status}"));
    // Mirrors `StorageMode`'s `Display` impl (`security/keyring_consent/types.rs`),
    // which is the source of truth. #6096 added the two `*_file` variants when it
    // stopped the file backends reporting themselves as `os_keyring` (#6076) — this
    // list is the whole enum, so a new variant fails here rather than silently
    // widening what the RPC may return.
    assert!(
        [
            "os_keyring",
            "local_encrypted",
            "local_encrypted_file",
            "local_plaintext_file",
            "consent_pending",
            "declined",
        ]
        .contains(&mode),
        "activeMode must be one of the six documented storage modes; got {mode:?}"
    );
    assert!(
        status
            .get("backendName")
            .and_then(Value::as_str)
            .is_some_and(|n| !n.is_empty()),
        "the backend must be named so the settings panel can show it: {status}"
    );
    assert_mode_matches_backend(status, "keyring_consent_status");
    if available {
        assert!(
            status.get("failureReason").is_none(),
            "`failureReason` is skipped when the keyring works: {status}"
        );
    } else {
        assert!(
            status.get("failureReason").is_some(),
            "an unavailable keyring must say why, or the settings panel has \
             nothing to explain to the user: {status}"
        );
    }

    // --- keyring_consent_decide --------------------------------------------
    let declined = harness
        .call(
            41,
            "openhuman.keyring_consent_decide",
            json!({ "mode": "declined" }),
        )
        .await;
    let declined_outer = assert_no_error(&declined, "keyring_consent_decide declined");
    assert!(
        logs(declined_outer)
            .iter()
            .any(|l| l.contains("keyring consent recorded: declined")),
        "the log must name the decision it recorded: {declined_outer}"
    );
    let declined = peel(declined_outer);
    assert_eq!(
        declined.get("storageMode").and_then(Value::as_str),
        Some("declined"),
        "the returned preference must echo the decision: {declined}"
    );
    let stamped = declined
        .get("consentedAtMs")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("the decision must be timestamped: {declined}"));
    assert!(
        stamped > 1_600_000_000_000,
        "the timestamp must be plausible epoch-ms, not 0 or a seconds value: {declined}"
    );

    // The decision reached disk. `decide` returns its preference *before* the
    // cache update, so without this the RPC could report success on a persist
    // that never happened and the choice would be silently lost on restart.
    assert!(
        stored_state.exists(),
        "keyring_consent_decide reported success but wrote no app-state.json at \
         {} — the preference exists only in memory and is lost on restart",
        stored_state.display()
    );
    let persisted = stored_consent(&stored_state);
    assert_eq!(
        persisted.get("storageMode").and_then(Value::as_str),
        Some("declined"),
        "the decision must be written to <workspace>/state/app-state.json: {persisted}"
    );
    assert_eq!(
        persisted.get("consentedAtMs").and_then(Value::as_u64),
        Some(stamped),
        "the persisted timestamp must be the same one the caller was handed, \
         not re-stamped on write: {persisted}"
    );

    // The opposite decision must overwrite it, so the assertion above is about
    // `decide` and not about a value that happened to be there already.
    let accepted = harness
        .call(
            42,
            "openhuman.keyring_consent_decide",
            json!({ "mode": "local_encrypted" }),
        )
        .await;
    assert_eq!(
        peel(assert_no_error(&accepted, "keyring_consent_decide local_encrypted"))
            .get("storageMode")
            .and_then(Value::as_str),
        Some("local_encrypted")
    );
    assert_eq!(
        stored_consent(&stored_state)
            .get("storageMode")
            .and_then(Value::as_str),
        Some("local_encrypted"),
        "a second decision must replace the first on disk, not append or be ignored"
    );

    // --- invalid input ------------------------------------------------------
    let invalid = harness
        .call(
            43,
            "openhuman.keyring_consent_decide",
            json!({ "mode": "os_keyring" }),
        )
        .await;
    let message = error_message(&invalid, "keyring_consent_decide invalid mode");
    assert!(
        message.contains("expected 'local_encrypted' or 'declined'"),
        "the rejection must list the modes a user may choose — `os_keyring` is \
         a *status*, not a decision: {message}"
    );
    assert_eq!(
        stored_consent(&stored_state)
            .get("storageMode")
            .and_then(Value::as_str),
        Some("local_encrypted"),
        "a rejected mode must not have been persisted before validation"
    );

    let no_mode = harness
        .call(44, "openhuman.keyring_consent_decide", json!({}))
        .await;
    assert!(
        error_message(&no_mode, "keyring_consent_decide no mode").contains("mode"),
        "a missing `mode` must name the parameter: {no_mode}"
    );

    // --- keyring_consent_retry_probe ---------------------------------------
    let probe = harness
        .call(45, "openhuman.keyring_consent_retry_probe", json!({}))
        .await;
    let probe_outer = assert_no_error(&probe, "keyring_consent_retry_probe");
    assert!(
        logs(probe_outer).iter().any(|l| l.contains("probe retried")),
        "the probe must report that it re-ran: {probe_outer}"
    );
    let probe = peel(probe_outer);
    assert!(
        probe.get("available").is_some_and(Value::is_boolean),
        "the probe returns a full status, same shape as `status`: {probe}"
    );
    assert!(
        probe
            .get("backendName")
            .and_then(Value::as_str)
            .is_some_and(|n| !n.is_empty()),
        "the probe result must name the backend it probed: {probe}"
    );
    assert_mode_matches_backend(probe, "keyring_consent_retry_probe");
}

// ── devices ──────────────────────────────────────────────────────────────────

/// `devices_list` and `devices_revoke` over the real SQLite store, and the
/// `devices_create_pairing` failure path when no tunnel is available.
#[tokio::test]
async fn devices_list_and_revoke_over_the_real_store() {
    let _lock = support::env_lock();
    let harness = Harness::start("", true).await;
    let config = config_for(&harness);

    // A fresh workspace lists nothing — and answers, rather than erroring on a
    // database that does not exist yet.
    let empty = harness.call(50, "openhuman.devices_list", json!({})).await;
    let empty = peel(assert_no_error(&empty, "devices_list empty"));
    assert_eq!(
        empty
            .get("devices")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0),
        "a workspace with no pairings must return an empty list, not an error: {empty}"
    );

    // Seed two paired devices directly — `devices_create_pairing` cannot do it
    // without a live backend tunnel.
    openhuman_core::openhuman::security::devices::store::insert_device(
        &config,
        "chan-alpha",
        "iPhone 15",
        "pubkey-alpha",
        "hash-alpha",
    )
    .expect("seed the first paired device");
    openhuman_core::openhuman::security::devices::store::insert_device(
        &config,
        "chan-beta",
        "iPad",
        "pubkey-beta",
        "hash-beta",
    )
    .expect("seed the second paired device");

    let listed = harness.call(51, "openhuman.devices_list", json!({})).await;
    let listed = peel(assert_no_error(&listed, "devices_list seeded"));
    let devices = listed
        .get("devices")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("devices_list must return a `devices` array: {listed}"));
    assert_eq!(devices.len(), 2, "both seeded devices must be listed: {listed}");

    let alpha = devices
        .iter()
        .find(|d| d.get("channel_id").and_then(Value::as_str) == Some("chan-alpha"))
        .unwrap_or_else(|| panic!("chan-alpha missing: {listed}"));
    assert_eq!(
        alpha.get("label").and_then(Value::as_str),
        Some("iPhone 15"),
        "the stored label must reach the caller: {alpha}"
    );
    assert_eq!(
        alpha.get("device_pubkey").and_then(Value::as_str),
        Some("pubkey-alpha")
    );
    assert_eq!(
        alpha.get("peer_online").and_then(Value::as_bool),
        Some(false),
        "`peer_online` is not persisted — it is overlaid from the in-memory \
         peer map, and must default to false rather than be absent: {alpha}"
    );
    assert_eq!(
        alpha.get("revoked").and_then(Value::as_bool),
        Some(false),
        "a freshly paired device is not revoked: {alpha}"
    );
    assert!(
        alpha
            .get("created_at")
            .and_then(Value::as_str)
            .is_some_and(|t| t.contains('T')),
        "created_at must be an ISO 8601 timestamp: {alpha}"
    );

    // --- devices_revoke -----------------------------------------------------
    let revoked = harness
        .call(
            52,
            "openhuman.devices_revoke",
            json!({ "channel_id": "chan-alpha" }),
        )
        .await;
    let revoked_outer = assert_no_error(&revoked, "devices_revoke");
    assert!(
        logs(revoked_outer)
            .iter()
            .any(|l| l.contains("chan-alpha") && l.contains("revoked")),
        "the log must name the channel it revoked: {revoked_outer}"
    );
    assert_eq!(
        peel(revoked_outer).get("success").and_then(Value::as_bool),
        Some(true),
        "revoking a live pairing must report success: {revoked_outer}"
    );

    // Revocation is a soft delete that `devices_list` filters out.
    let after = harness.call(53, "openhuman.devices_list", json!({})).await;
    let after = peel(assert_no_error(&after, "devices_list after revoke"));
    let remaining = after
        .get("devices")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("devices array missing: {after}"));
    assert_eq!(
        remaining.len(),
        1,
        "a revoked device must disappear from the list: {after}"
    );
    assert_eq!(
        remaining[0].get("channel_id").and_then(Value::as_str),
        Some("chan-beta"),
        "revoking chan-alpha must not touch chan-beta: {after}"
    );
    // The row still exists, revoked — the list filters, it does not delete.
    let row = openhuman_core::openhuman::security::devices::store::get_device(
        &config,
        "chan-alpha",
    )
    .expect("read the revoked row back");
    assert!(
        row.is_some_and(|d| d.revoked),
        "revoke is documented as a soft delete; the row must survive with \
         `revoked = 1` so the pairing history is auditable"
    );

    // Revoking something that was never paired must report `false`, not error.
    let unknown = harness
        .call(
            54,
            "openhuman.devices_revoke",
            json!({ "channel_id": "chan-never-existed" }),
        )
        .await;
    assert_eq!(
        peel(assert_no_error(&unknown, "devices_revoke unknown"))
            .get("success")
            .and_then(Value::as_bool),
        Some(false),
        "revoking an unknown channel must report that nothing was revoked, \
         rather than claiming success: {unknown}"
    );

    // A missing channel_id is a parameter error.
    let no_id = harness.call(55, "openhuman.devices_revoke", json!({})).await;
    assert!(
        error_message(&no_id, "devices_revoke no channel_id").contains("channel_id"),
        "the error must name the missing parameter: {no_id}"
    );
}

/// Pairing needs the shared backend socket. Without it the controller must fail
/// with a reason that points at the tunnel, and must not leave a half-built
/// pairing behind in the device list.
#[tokio::test]
async fn devices_create_pairing_fails_without_a_backend_tunnel() {
    let _lock = support::env_lock();
    let harness = Harness::start("", true).await;

    let pairing = harness
        .call(
            60,
            "openhuman.devices_create_pairing",
            json!({ "label": "iPhone" }),
        )
        .await;
    let message = error_message(&pairing, "devices_create_pairing with no socket");
    assert!(
        message.contains("[devices/tunnel]"),
        "the failure must be attributed to the tunnel, so the user is not sent \
         hunting through the pairing UI; got: {message}"
    );

    // Nothing may be persisted from a pairing that never completed a handshake.
    let listed = harness.call(61, "openhuman.devices_list", json!({})).await;
    assert_eq!(
        peel(assert_no_error(&listed, "devices_list after failed pairing"))
            .get("devices")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0),
        "a failed pairing must not leave a device row behind: {listed}"
    );
}

// ── wallet ───────────────────────────────────────────────────────────────────

/// `wallet_encode_erc20_transfer` builds `transfer(address,uint256)` calldata.
/// The selector and both padded words are checked byte for byte — an encoder
/// that produced *some* hex string would pass a "not empty" test and fail this.
#[tokio::test]
async fn wallet_encode_erc20_transfer_produces_exact_calldata() {
    let _lock = support::env_lock();
    let harness = Harness::start("", true).await;

    let encoded = harness
        .call(
            70,
            "openhuman.wallet_encode_erc20_transfer",
            json!({
                "chain": "evm",
                "toAddress": "0x9858EfFD232B4033E47d90003D41EC34EcaEda94",
                "amountRaw": "1000000000000000000"
            }),
        )
        .await;
    let calldata = peel(assert_no_error(&encoded, "wallet_encode_erc20_transfer"))
        .as_str()
        .unwrap_or_else(|| panic!("calldata must be a string: {encoded}"))
        .to_string();

    assert!(
        calldata.starts_with("0xa9059cbb"),
        "`transfer(address,uint256)` has selector 0xa9059cbb: {calldata}"
    );
    assert_eq!(
        calldata.len(),
        2 + 8 + 64 + 64,
        "0x + 4-byte selector + two 32-byte words: {calldata}"
    );
    assert!(
        calldata[10..74].ends_with("9858effd232b4033e47d90003d41ec34ecaeda94"),
        "the recipient must be left-padded into the first word, lower-cased: {calldata}"
    );
    assert!(
        calldata[10..74].starts_with(&"0".repeat(24)),
        "the address word must be zero-padded to 32 bytes: {calldata}"
    );
    assert_eq!(
        &calldata[74..],
        "0000000000000000000000000000000000000000000000000de0b6b3a7640000",
        "1e18 must encode as the big-endian 32-byte amount word: {calldata}"
    );

    // Failure paths — each names what the caller got wrong, because an agent
    // reads these messages to correct itself.
    let bad_address = harness
        .call(
            71,
            "openhuman.wallet_encode_erc20_transfer",
            json!({ "chain": "evm", "toAddress": "not-an-address", "amountRaw": "1" }),
        )
        .await;
    let message = error_message(&bad_address, "encode with a bad address");
    assert!(
        message.contains("invalid EVM recipient address") && message.contains("not-an-address"),
        "the rejection must quote the address it rejected: {message}"
    );

    let bad_amount = harness
        .call(
            72,
            "openhuman.wallet_encode_erc20_transfer",
            json!({
                "chain": "evm",
                "toAddress": "0x9858EfFD232B4033E47d90003D41EC34EcaEda94",
                "amountRaw": "1.5"
            }),
        )
        .await;
    assert!(
        error_message(&bad_amount, "encode with a fractional amount")
            .contains("is not a valid non-negative integer"),
        "`amountRaw` is the token's smallest unit, so a decimal must be \
         rejected with the documented wording: {bad_amount}"
    );

    let wrong_chain = harness
        .call(
            73,
            "openhuman.wallet_encode_erc20_transfer",
            json!({
                "chain": "solana",
                "toAddress": "0x9858EfFD232B4033E47d90003D41EC34EcaEda94",
                "amountRaw": "1"
            }),
        )
        .await;
    assert!(
        error_message(&wrong_chain, "encode on a non-EVM chain")
            .contains("only supports the evm chain"),
        "ERC-20 calldata on a non-EVM chain must be refused: {wrong_chain}"
    );
}

/// `wallet_reveal_recovery_phrase` decrypts the stored mnemonic. Driven through
/// the RPC surface end to end: encrypt the phrase, set the wallet up with it,
/// then reveal it back.
#[tokio::test]
async fn wallet_reveal_recovery_phrase_returns_the_stored_mnemonic() {
    let _lock = support::env_lock();
    let harness = Harness::start("", true).await;

    // Before setup there is nothing to reveal, and the message has to tell the
    // user what to do about it.
    let before = harness
        .call(80, "openhuman.wallet_reveal_recovery_phrase", json!({}))
        .await;
    let message = error_message(&before, "reveal before setup");
    assert!(
        message.contains("No recovery phrase is available"),
        "an unconfigured wallet must say so plainly: {message}"
    );
    assert!(
        message.contains("Set up or unlock your wallet first"),
        "the message must tell the user the next step: {message}"
    );

    const MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon \
                            abandon abandon abandon abandon abandon about";
    let normalized = MNEMONIC.split_whitespace().collect::<Vec<_>>().join(" ");

    openhuman_core::openhuman::security::keyring::init_workspace(&harness.workspace());
    let encrypted = harness
        .call(
            81,
            "openhuman.encrypt_secret",
            json!({ "plaintext": normalized }),
        )
        .await;
    let encrypted_mnemonic = peel(assert_no_error(&encrypted, "encrypt the mnemonic"))
        .as_str()
        .expect("ciphertext")
        .to_string();

    let setup = harness
        .call(
            82,
            "openhuman.wallet_setup",
            json!({
                "consentGranted": true,
                "source": "generated",
                "mnemonicWordCount": 12,
                "encryptedMnemonic": encrypted_mnemonic,
                // All four chains: `wallet_setup` rejects a partial account set
                // ("wallet setup must include exactly one 'btc' account").
                "accounts": [
                    { "chain": "evm", "address": "0x9858EfFD232B4033E47d90003D41EC34EcaEda94", "derivationPath": "m/44'/60'/0'/0/0" },
                    { "chain": "btc", "address": "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu", "derivationPath": "m/84'/0'/0'/0/0" },
                    { "chain": "solana", "address": "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk", "derivationPath": "m/44'/501'/0'/0'" },
                    { "chain": "tron", "address": "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH", "derivationPath": "m/44'/195'/0'/0/0" }
                ]
            }),
        )
        .await;
    let setup = peel(assert_no_error(&setup, "wallet_setup"));
    assert_eq!(
        setup.get("configured").and_then(Value::as_bool),
        Some(true),
        "the wallet must be configured before reveal can mean anything: {setup}"
    );
    assert_eq!(
        setup.get("accounts").and_then(Value::as_array).map(Vec::len),
        Some(4),
        "all four chains must be persisted: {setup}"
    );

    let revealed = harness
        .call(83, "openhuman.wallet_reveal_recovery_phrase", json!({}))
        .await;
    let revealed_outer = assert_no_error(&revealed, "wallet_reveal_recovery_phrase");
    assert!(
        logs(revealed_outer)
            .iter()
            .any(|l| l == "recovery phrase revealed"),
        "the outcome must carry its log line: {revealed_outer}"
    );
    let revealed = peel(revealed_outer);

    assert_eq!(
        revealed.get("phrase").and_then(Value::as_str),
        Some(normalized.as_str()),
        "reveal must return the exact phrase that was stored — a wrong or \
         re-derived phrase would lose the user's funds: {revealed}"
    );
    assert_eq!(
        revealed.get("wordCount").and_then(Value::as_u64),
        Some(12),
        "the word count must be counted from the decrypted phrase, not echoed \
         from the setup parameter: {revealed}"
    );
}
