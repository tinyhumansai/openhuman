pub mod ops;
mod schemas;
pub mod tools;

pub mod audit;
pub mod audit_record;
// Kernel security family (step 7 of the domain reorg). These seven domains are
// the physical form of `kernel.md` §3.4: `SecurityPolicy`, the approval gate,
// taint/redaction, credentials and the keychain are exactly the set a future
// `Guard<D>` decorator draws from, and a swapped driver must never see any of
// it. None of them is gated — the family is kernel, permanently.
pub mod approval;
pub mod credentials;
pub mod devices;
pub mod egress;
pub mod encryption;
pub mod keyring_consent;
pub mod live_policy;
pub mod pairing;
pub mod policy;
pub mod prompt_injection;
pub mod scrub;
pub mod secret_storage;
pub mod switch;

#[allow(unused_imports)]
pub use self::secret_storage::SecretStore;
#[allow(unused_imports)]
pub use audit::{get_or_create_workspace_audit_sink, AuditSink};
#[allow(unused_imports)]
pub use egress::{
    emit_external_transfer, enforce_egress, local_only_tool_block, DataKind, EgressDescriptor,
    EgressReason, IdentificationRisk,
};
pub use ops as rpc;
pub use ops::*;
#[allow(unused_imports)]
pub use pairing::is_public_bind;
pub use policy::validate_path_within_root;
#[allow(unused_imports)]
pub use policy::AutonomyLevel;
#[allow(unused_imports)]
pub use policy::GateDecision;
pub use policy::SecurityPolicy;
pub use policy::ToolOperation;
pub use policy::{ensure_openhuman_scratch_dir, openhuman_scratch_dir};
#[allow(unused_imports)]
pub use policy::{TrustedAccess, TrustedRoot};

pub use schemas::{
    all_controller_schemas as all_security_controller_schemas,
    all_registered_controllers as all_security_registered_controllers,
};

/// Declare that the secret state of the config directory `config_dir` does not
/// outlive the runtime using it (an embedder's scratch directory).
///
/// TinySecurity boots with a neutral secret authority, and each config
/// directory registers its own approval scope and secret registration on it
/// at runtime (its own cipher, consumer, delivery slots and audit subject).
/// A declared directory's registration is retired at the next
/// [`release_secret_scope`]; a durable one keeps it for a later runtime over
/// the same directory. Declarations last until the next release. A no-op in
/// a build without the `security-module` feature.
pub fn declare_transient_secret_scope(config_dir: &std::path::Path) {
    #[cfg(feature = "security-module")]
    crate::modules::security::declare_transient_secret_scope(config_dir);
    #[cfg(not(feature = "security-module"))]
    let _ = config_dir;
}

/// Retire the secret registration of the config directory `config_dir`: a
/// profile closed or evicted from this process. The module drops its master,
/// cipher and pending work at the next security lifecycle call; reopening the
/// profile registers it again under fresh registration ids. A no-op in a
/// build without the `security-module` feature.
pub fn retire_secret_scope(config_dir: &std::path::Path) {
    #[cfg(feature = "security-module")]
    crate::modules::security::retire_secret_scope(config_dir);
    #[cfg(not(feature = "security-module"))]
    let _ = config_dir;
}

/// Release the secret scopes of a host whose core is gone: every scope
/// declared with [`declare_transient_secret_scope`] is retired at the next
/// security lifecycle call. Other runtimes' registrations are untouched. A
/// no-op in a build without the `security-module` feature.
pub fn release_secret_scope() {
    #[cfg(feature = "security-module")]
    crate::modules::security::release_secret_scope();
}
