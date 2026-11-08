//! The single translation point for TinySecurity bootstrap policy.

use crate::config::Config;

/// The host requires the audited native permission vocabulary from its contract.
pub const MODULE_CONTRACT_VERSION: (u32, u32) = tinysecurity_bus::CONTRACT_VERSION;

/// Check the exact published artifact contract this adapter was built for.
#[must_use]
pub const fn supports_contract(version: (u32, u32)) -> bool {
    version.0 == MODULE_CONTRACT_VERSION.0 && version.1 == MODULE_CONTRACT_VERSION.1
}

/// Translate OpenHuman autonomy settings to the published bus contract.
#[must_use]
pub fn policy_config(config: &Config) -> tinysecurity_bus::PolicyConfig {
    tinysecurity_bus::PolicyConfig {
        enabled: config.autonomy.enabled,
        action_dir: config.action_dir.display().to_string(),
        workspace_dir: config.workspace_dir.display().to_string(),
        // These settings are inert while autonomy is disabled. Do not send
        // host defaults that the module must reject despite having no effect.
        allowed_commands: if config.autonomy.enabled {
            config.autonomy.allowed_commands.clone()
        } else {
            Vec::new()
        },
        forbidden_paths: if config.autonomy.enabled {
            forbidden_paths(&config.autonomy.forbidden_paths, cfg!(windows))
        } else {
            Vec::new()
        },
    }
}

/// The forbidden paths as the module receives them, `~` expanded. On Windows
/// a path rooted without a drive (`/etc`, `\usr`, the Unix roots in the
/// shipped defaults) is not absolute and names no system folder; the module
/// already ignores such entries there, and `validate` would otherwise refuse
/// the whole policy for them, so they are left out. Relative entries are
/// kept, so `validate` still refuses them.
fn forbidden_paths(paths: &[String], windows: bool) -> Vec<String> {
    paths
        .iter()
        .filter(|path| !(windows && is_drive_less_root(path)))
        .map(|path| crate::config::expand_tilde(path))
        .collect()
}

/// `/x` or `\x`, but not a UNC path (`\\server`, `//server`).
fn is_drive_less_root(path: &str) -> bool {
    path.starts_with(['/', '\\']) && !path.starts_with("\\\\") && !path.starts_with("//")
}

/// Reject enabled host policy that the published module contract cannot
/// represent. Disabled policy settings are inert and serialize empty above.
pub fn validate(config: &Config) -> Result<(), &'static str> {
    if !config.autonomy.enabled {
        return Ok(());
    }
    if !config.action_dir.is_absolute() || !config.workspace_dir.is_absolute() {
        return Err("enabled policy roots must be absolute paths");
    }
    if config
        .autonomy
        .allowed_commands
        .iter()
        .any(|command| command.is_empty() || command.chars().any(char::is_whitespace))
    {
        return Err("enabled policy allows only argument-free command names");
    }
    if policy_config(config)
        .forbidden_paths
        .iter()
        .any(|path| !std::path::Path::new(path).is_absolute())
    {
        return Err("enabled policy forbidden paths must be absolute");
    }
    Ok(())
}

/// Complete SDK bootstrap payload used both by first load and reinitialization.
#[must_use]
pub fn module_config(config: &Config) -> tinysecurity_bus::ModuleConfig {
    tinysecurity_bus::ModuleConfig {
        policy: policy_config(config),
        approvals: Some(approval_init_config()),
        sandbox: Some(tinysecurity_bus::SandboxConfig2 { scopes: Vec::new() }),
        // Sandbox plans require a durable audit correlation even when the
        // operator disables the optional general audit feature.
        audit: Some(audit_init_config(config)),
        crypto: Some(tinysecurity_bus::CryptoConfig2 {
            max_payload_bytes: tinysecurity_bus::MAX_CRYPTO_PAYLOAD_BYTES,
            operations_per_minute: 120,
            keyring: module_keyring_backend(
                std::env::var("OPENHUMAN_KEYRING_BACKEND").ok().as_deref(),
                &config.workspace_dir,
            )
            .map(|backend| tinysecurity_bus::KeyringConfig2 {
                service: "openhuman".into(),
                backend,
                consent_timeout_ms: KEYRING_CONSENT_TIMEOUT_MS,
            }),
            pairing: None,
        }),
        ..tinysecurity_bus::ModuleConfig::default()
    }
}

/// TinySecurity's keyring store for the host's `OPENHUMAN_KEYRING_BACKEND`
/// selection, parsed exactly as the host's own secret storage parses it.
///
/// - `file` / `encrypted_file`: the host keeps its secrets in files under the
///   workspace (`secrets.enc`, `keyring/master.key`), typically because the
///   machine has no usable OS keyring. The module's store goes beside them in
///   `keyring/tinysecurity`, encrypted under its own key file.
/// - `os`, unset or unrecognised: the OS keyring, as before.
/// - `memory`: no module keyring at all, so every keyring call is refused.
///   The operator asked for secrets to stay out of durable storage; writing
///   them to the OS keyring instead would quietly persist them.
///
/// A relative workspace also gets no keyring: the module refuses a relative
/// store directory at load, which would deny the whole security module rather
/// than only its keyring.
///
/// TinySecurity's crypto settings are immutable once loaded, so a selection
/// that changes after the first load makes a reinitialization fail closed.
pub(crate) fn module_keyring_backend(
    selection: Option<&str>,
    workspace_dir: &std::path::Path,
) -> Option<tinysecurity_bus::KeyringBackend2> {
    use crate::security::secret_storage::store::{backend_kind_from_env_value, BackendKind};
    use tinysecurity_bus::KeyringBackend2;

    match selection.and_then(backend_kind_from_env_value) {
        Some(BackendKind::File | BackendKind::EncryptedFile) => {
            if !workspace_dir.is_absolute() {
                log::warn!(
                    "[security] module keyring refused: file keyring needs an absolute workspace"
                );
                return None;
            }
            let directory = workspace_dir.join("keyring").join("tinysecurity");
            log::debug!(
                "[security] module keyring backend=encrypted_file dir={}",
                directory.display()
            );
            Some(KeyringBackend2::EncryptedFile {
                directory: directory.display().to_string(),
            })
        }
        Some(BackendKind::Memory) => {
            log::debug!("[security] module keyring refused: memory keyring backend selected");
            None
        }
        Some(BackendKind::Host) | None => Some(KeyringBackend2::Os),
    }
}

/// TinySecurity's deadline for the host's answer to one keyring prompt. It
/// outlasts the host's own wait, so an unanswered prompt comes back as
/// pending from the host rather than racing the module's timer.
pub(crate) const KEYRING_CONSENT_TIMEOUT_MS: u32 =
    crate::security::keyring_consent::operation_approval::MAX_CONSENT_WAIT_MS + 5_000;
// TinySecurity accepts up to 300 s.
const _: () = assert!(KEYRING_CONSENT_TIMEOUT_MS <= 300_000);

const AUDIT_CALLBACK_TIMEOUT_MS: u32 = 20_000;

/// Init-time subject is deliberately separate from every registered user
/// subject. Real audit rows are sent through `RecordAudit2` to the per-subject
/// stream registered by `security::approval::native`.
fn bootstrap_subject() -> tinysecurity_bus::SubjectScope {
    tinysecurity_bus::SubjectScope {
        user: "openhuman".into(),
        workspace: "bootstrap".into(),
        agent: "bootstrap".into(),
    }
}

fn approval_init_config() -> tinysecurity_bus::ApprovalConfig2 {
    tinysecurity_bus::ApprovalConfig2 {
        timeout_ms: AUDIT_CALLBACK_TIMEOUT_MS,
        scopes: vec![tinysecurity_bus::ApprovalScope2 {
            scope_id: "0".repeat(32),
            subject: bootstrap_subject(),
            policy_ref: "openhuman/approval/bootstrap".into(),
            generation: 1,
            ceiling: tinysecurity_bus::AccessTier::Read,
            require_approval: true,
            auto_approve_all: false,
        }],
    }
}

/// The subject and stream of TinySecurity's init audit client: the one stream
/// the module appends every audited permission (path, command, egress) to,
/// whichever tenant asked. The host correlates those calls on this stream
/// ([`crate::security::approval::native::permission_audit`]), never on a
/// tenant's own approval stream, or the module's batch names a stream the
/// sink does not know and every audited permission is denied.
pub(crate) fn init_audit_identity() -> (tinysecurity_bus::SubjectScope, String) {
    (bootstrap_subject(), "b".repeat(32))
}

fn audit_init_config(config: &Config) -> tinysecurity_bus::AuditConfig2 {
    let max_bytes = u64::from(config.audit.max_size_mb)
        .saturating_mul(1024 * 1024)
        .max(1);
    let (scope, stream_id) = init_audit_identity();
    tinysecurity_bus::AuditConfig2 {
        scope,
        stream_id,
        timeout_ms: AUDIT_CALLBACK_TIMEOUT_MS,
        required: false,
        native_path: None,
        max_bytes,
        max_archives: 3,
        journal_max_bytes: max_bytes,
        journal_max_events: 10_000,
    }
}

/// Complete SDK bootstrap payload with the host identity required by the
/// module's authenticated bus boundary.
pub fn module_config_for_host(
    config: &Config,
    trusted_host: Option<String>,
    judge: Option<tinysecurity_bus::JudgeConfig>,
) -> Result<tinysecurity_bus::ModuleConfig, &'static str> {
    let trusted_host = trusted_host.ok_or("module host identity unavailable")?;
    Ok(tinysecurity_bus::ModuleConfig {
        contract_version: MODULE_CONTRACT_VERSION,
        trusted_host: Some(trusted_host),
        judge,
        ..module_config(config)
    })
}

/// The neutral TinySecurity bootstrap: the secret authority and the device
/// pairing owner exist, but no tenant's secret scope is configured at init.
///
/// Every tenant (a desktop user, a SaaS profile, an embedder's scratch
/// directory) registers its own approval scope and secret registration at
/// runtime ([`secret_scope_request`]), each with its own routine cipher,
/// consumer, delivery slots and audit subject (contract 2.1). A second
/// tenant in the same process is therefore never handed the first one's
/// master. The init audit stays required: TinySecurity only binds a runtime
/// cipher to an audit client whose delivery is required.
pub(crate) fn module_config_for_secret_authority(
    config: &Config,
    trusted_host: Option<String>,
    judge: Option<tinysecurity_bus::JudgeConfig>,
) -> Result<tinysecurity_bus::ModuleConfig, &'static str> {
    use tinysecurity_bus::{Origin, SecretConfig2, SecretLimits2};

    let mut result = module_config_for_host(config, trusted_host, judge)?;
    if let Some(audit) = result.audit.as_mut() {
        audit.required = true;
    }
    result.secret = Some(SecretConfig2 {
        registrations: Vec::new(),
        limits: SecretLimits2::default(),
        callback_timeout_ms: AUDIT_CALLBACK_TIMEOUT_MS,
    });
    result.secret_cipher = None;
    result.device = Some(tinysecurity_bus::DeviceConfig2 {
        scopes: Vec::new(),
        max_handles: 512,
        max_in_flight: 32,
        operations_per_minute: 120,
        frames_per_minute: 12_000,
        max_ttl_ms: 300_000,
        max_message_bytes: 1_048_576,
        max_bootstrap_nonces: 4096,
    });
    result.device_pairing = Some(tinysecurity_bus::DevicePairingConfig2 {
        scopes: Vec::new(),
        runtime_scope: Some(tinysecurity_bus::DevicePairingRuntimeScope2 {
            persistence_backend_id: pairing_backend_id(),
            legacy_device_backend_id: Some(pairing_legacy_backend_id()),
            allowed_origins: vec![Origin::Operator],
        }),
        max_pairings: 256,
        max_pending: 32,
        max_pending_ttl_ms: 120_000,
        max_session_ttl_ms: 604_800_000,
        max_state_bytes: 1_048_576,
        callback_timeout_ms: AUDIT_CALLBACK_TIMEOUT_MS,
    });
    Ok(result)
}

/// The opaque identifiers of one tenant's runtime secret registration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SecretRegistrationIds {
    pub(crate) registration_id: String,
    pub(crate) consumer_id: String,
    pub(crate) target_backend_id: String,
    pub(crate) legacy_backend_id: String,
}

/// The routine cipher's receiver slots per tenant registration.
const CIPHER_SLOTS: u16 = 8;

/// `RegisterSecretScope2` for one tenant: its fixed config-master
/// registration under its own approval scope, with a cipher bound to that
/// registration alone and audited on the stream its approval scope owns.
pub(crate) fn secret_scope_request(
    scope: &crate::security::approval::native::SecretScope,
    ids: &SecretRegistrationIds,
) -> tinysecurity_bus::RegisterSecretScopeRequest2 {
    use tinysecurity_bus::{
        MasterPurpose2, MasterSlotKind2, Origin, SecretBinding2, SecretRegistration2,
        SecretScopeCipher2,
    };

    tinysecurity_bus::RegisterSecretScopeRequest2 {
        registration: SecretRegistration2 {
            registration_id: ids.registration_id.clone(),
            approval_scope_id: scope.scope_id().to_owned(),
            origin: Origin::Operator,
            purpose: MasterPurpose2::ConfigUserMaster,
            target: SecretBinding2 {
                backend_id: ids.target_backend_id.clone(),
                slot: MasterSlotKind2::User,
            },
            legacy: vec![SecretBinding2 {
                backend_id: ids.legacy_backend_id.clone(),
                slot: MasterSlotKind2::File,
            }],
        },
        cipher: Some(SecretScopeCipher2 {
            consumer_id: ids.consumer_id.clone(),
            enabled: true,
            slots: CIPHER_SLOTS,
            timeout_ms: AUDIT_CALLBACK_TIMEOUT_MS,
            audit_stream_id: scope.stream_id().to_owned(),
        }),
    }
}

fn process_id(label: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(label.as_bytes()))[..32].to_owned()
}

/// The device-pairing persistence backend every runtime pairing scope of this
/// process shares; each document stays bound to its own secret registration.
pub(super) fn pairing_backend_id() -> String {
    process_id("device-pairing-state:runtime-scopes")
}

/// The legacy device-identity source every runtime pairing scope shares.
pub(super) fn pairing_legacy_backend_id() -> String {
    process_id("device-pairing-legacy:runtime-scopes")
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
