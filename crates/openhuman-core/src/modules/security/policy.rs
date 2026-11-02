//! The single translation point for TinySecurity bootstrap policy.

use crate::config::Config;

/// Wire contract served by the checksum-pinned v0.18.0 module. The fork bus
/// contains newer host APIs, but this adapter invokes only this exact subset.
pub const MODULE_CONTRACT_VERSION: (u32, u32) = (1, 33);

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
            config.autonomy.forbidden_paths.clone()
        } else {
            Vec::new()
        },
    }
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
    if config
        .autonomy
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
            keyring: Some(tinysecurity_bus::KeyringConfig2 {
                service: "openhuman".into(),
                backend: tinysecurity_bus::KeyringBackend2::Os,
                consent_timeout_ms: 120_000,
            }),
            pairing: None,
        }),
        ..tinysecurity_bus::ModuleConfig::default()
    }
}

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

fn audit_init_config(config: &Config) -> tinysecurity_bus::AuditConfig2 {
    let max_bytes = u64::from(config.audit.max_size_mb)
        .saturating_mul(1024 * 1024)
        .max(1);
    tinysecurity_bus::AuditConfig2 {
        scope: bootstrap_subject(),
        stream_id: "b".repeat(32),
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
) -> Result<tinysecurity_bus::ModuleConfig, &'static str> {
    let trusted_host = trusted_host.ok_or("module host identity unavailable")?;
    Ok(tinysecurity_bus::ModuleConfig {
        contract_version: MODULE_CONTRACT_VERSION,
        trusted_host: Some(trusted_host),
        ..module_config(config)
    })
}

/// Add the exact persisted approval/audit scope and fixed config-master
/// registration to this candidate's initialization config.
pub(crate) fn module_config_for_secret_scope(
    config: &Config,
    trusted_host: Option<String>,
    scope: &crate::security::approval::native::SecretScope,
    registration_id: String,
    consumer_id: String,
    target_backend_id: String,
    legacy_backend_id: String,
    pairing_backend_id: String,
) -> Result<tinysecurity_bus::ModuleConfig, &'static str> {
    use tinysecurity_bus::{
        AccessTier, ApprovalScope2, MasterPurpose2, MasterSlotKind2, Origin, SecretBinding2,
        SecretCipherConfig2, SecretConfig2, SecretLimits2, SecretRegistration2,
    };

    let mut result = module_config_for_host(config, trusted_host)?;
    let approval = result.approvals.get_or_insert_with(approval_init_config);
    approval.scopes.push(ApprovalScope2 {
        scope_id: scope.scope_id().to_owned(),
        subject: scope.subject().clone(),
        policy_ref: "openhuman/approval/v1".into(),
        generation: 1,
        ceiling: AccessTier::Full,
        require_approval: true,
        auto_approve_all: false,
    });
    result.audit = Some(tinysecurity_bus::AuditConfig2 {
        scope: scope.subject().clone(),
        stream_id: scope.stream_id().to_owned(),
        timeout_ms: AUDIT_CALLBACK_TIMEOUT_MS,
        required: true,
        native_path: None,
        max_bytes: 10 * 1024 * 1024,
        max_archives: 3,
        journal_max_bytes: 10 * 1024 * 1024,
        journal_max_events: 10_000,
    });
    result.secret = Some(SecretConfig2 {
        registrations: vec![SecretRegistration2 {
            registration_id: registration_id.clone(),
            approval_scope_id: scope.scope_id().to_owned(),
            origin: Origin::Operator,
            purpose: MasterPurpose2::ConfigUserMaster,
            target: SecretBinding2 {
                backend_id: target_backend_id,
                slot: MasterSlotKind2::User,
            },
            legacy: vec![SecretBinding2 {
                backend_id: legacy_backend_id,
                slot: MasterSlotKind2::File,
            }],
        }],
        limits: SecretLimits2::default(),
        callback_timeout_ms: AUDIT_CALLBACK_TIMEOUT_MS,
    });
    result.secret_cipher = Some(SecretCipherConfig2 {
        registration_id: registration_id.clone(),
        consumer_id,
        enabled: true,
        slots: 8,
        timeout_ms: AUDIT_CALLBACK_TIMEOUT_MS,
    });
    let pairing_legacy_backend_id = pairing_legacy_backend_id(&registration_id);
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
            persistence_backend_id: pairing_backend_id,
            legacy_device_backend_id: Some(pairing_legacy_backend_id),
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

pub(super) fn pairing_legacy_backend_id(registration_id: &str) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{:x}",
        Sha256::digest(format!("device-pairing-legacy:{registration_id}").as_bytes())
    )[..32]
        .to_owned()
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
