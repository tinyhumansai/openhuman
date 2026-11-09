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
/// TinySecurity keeps the secret scope it was admitted with (approval subject,
/// audit stream and master-key registration, all derived from the admitting
/// config) for as long as the module is loaded, which is the life of the
/// process. After [`release_secret_scope`], a later config may take that scope
/// over only if it is the admitting config itself or declared here: a durable
/// config would otherwise keep secrets encrypted under another scope's master
/// key. Declarations last until the next release. A no-op in a build without
/// the `security-module` feature.
pub fn declare_transient_secret_scope(config_dir: &std::path::Path) {
    #[cfg(feature = "security-module")]
    crate::modules::security::declare_transient_secret_scope(config_dir);
    #[cfg(not(feature = "security-module"))]
    let _ = config_dir;
}

/// Release the secret scope using the admitted TinySecurity module, for a host
/// whose core is gone.
///
/// While the core that admitted TinySecurity is live, a config that derives a
/// different secret scope is refused the module, so one live scope is never
/// handed another's master key. A host that releases its core and builds a
/// new one in the same process (an embedder's `Runtime`, each in its own
/// scratch directory) calls this once the old core is dropped, so the next
/// core can take the admitted module over (see
/// [`declare_transient_secret_scope`]) instead of being refused every security
/// call. A load still in flight for the released core does not reclaim it. A
/// no-op in a build without the `security-module` feature.
pub fn release_secret_scope() {
    #[cfg(feature = "security-module")]
    crate::modules::security::release_secret_scope();
}
