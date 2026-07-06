//! Emergency stop — a fail-closed kill switch for desktop automation.
//!
//! `EmergencyStop` is a process-global switch (mirrors `ApprovalGate`). When
//! engaged, the tinyagents approval middleware refuses external-effect tool
//! calls and `accessibility_input_action` refuses clicks/typing, until the
//! user resumes. Engaging also stops the accessibility session and
//! cascade-denies pending approvals. In-memory only (resets on restart).

pub mod ops;
pub mod schemas;
pub mod state;
pub mod types;

pub use schemas::{all_emergency_controller_schemas, all_emergency_registered_controllers};
pub use state::{is_engaged_global, EmergencyStop};
pub use types::HaltState;
