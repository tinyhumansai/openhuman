pub mod ops;
mod schemas;
pub mod tools;

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
pub mod floor;
pub mod keyring_consent;
pub mod live_policy;
pub mod pairing;
pub mod policy;
pub mod prompt_injection;
pub mod redact;
pub mod secret_storage;
pub(crate) mod secure_open;
pub(crate) mod transport;
pub mod turn_origin;

pub use self::secret_storage::SecretStore;
pub use egress::{
    emit_external_transfer, enforce_egress, local_only_tool_block, DataKind, EgressDescriptor,
    EgressReason, IdentificationRisk,
};
pub use ops as rpc;
pub use ops::*;
pub use policy::validate_path_within_root;
pub use policy::AutonomyLevel;
pub use policy::GateDecision;
pub use policy::SecurityPolicy;
pub use policy::{ensure_openhuman_scratch_dir, openhuman_scratch_dir};
pub use policy::{Budget, PathKind};
pub use policy::{TrustedAccess, TrustedRoot};
pub use {
    pairing::ensure_core_rpc_token_for_bind, pairing::is_public_bind, pairing::CoreBindTokenError,
    pairing::CORE_TOKEN_ENV_VAR,
};

pub use schemas::{
    all_controller_schemas as all_security_controller_schemas,
    all_registered_controllers as all_security_registered_controllers,
};
