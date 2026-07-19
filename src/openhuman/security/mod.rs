mod core;
pub mod ops;
mod schemas;
pub mod tools;

pub mod audit;
pub mod bubblewrap;
pub mod detect;
pub mod docker;
pub mod egress;
pub mod firejail;
pub mod landlock;
pub mod live_policy;
pub mod pairing;
pub mod pii;
pub mod policy;
pub mod secrets;
pub mod traits;

#[allow(unused_imports)]
pub use crate::openhuman::keyring::SecretStore;
#[allow(unused_imports)]
pub use audit::{
    get_or_create_workspace_audit_logger, AuditEvent, AuditEventType, AuditLogger,
    CommandExecutionLog,
};
pub use core::*;
#[allow(unused_imports)]
pub use detect::create_sandbox;
#[allow(unused_imports)]
pub use egress::{
    emit_external_transfer, enforce_egress, local_only_blocks, local_only_tool_block, DataKind,
    EgressDescriptor, EgressReason, IdentificationRisk,
};
pub use ops as rpc;
pub use ops::*;
#[allow(unused_imports)]
pub use pairing::{
    ensure_core_rpc_token_for_bind, is_public_bind, CoreBindTokenError, PairingGuard,
    CORE_TOKEN_ENV_VAR,
};
pub use policy::validate_path_within_root;
#[allow(unused_imports)]
pub use policy::AutonomyLevel;
pub use policy::SecurityPolicy;
pub use policy::ToolOperation;
pub use policy::{ensure_openhuman_scratch_dir, openhuman_scratch_dir};
#[allow(unused_imports)]
pub use policy::{CommandClass, GateDecision};
#[allow(unused_imports)]
pub use policy::{TrustedAccess, TrustedRoot};
pub use policy::{POLICY_BLOCKED_MARKER, POLICY_DENIED_MARKER};
#[allow(unused_imports)]
pub use traits::{NoopSandbox, Sandbox};

pub use schemas::{
    all_controller_schemas as all_security_controller_schemas,
    all_registered_controllers as all_security_registered_controllers,
};

#[allow(unused_imports)]
pub use pii::{scan as scan_pii, CategoryHit, PiiCategory, PiiScanResult, RiskLevel};
