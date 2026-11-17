use super::common::{opt, out};
use crate::core::{ControllerSchema, TypeSchema};

pub(super) fn schema(function: &str) -> Option<ControllerSchema> {
    match function {
        "migration_scan" => Some(ControllerSchema {
            namespace: "memory",
            function: "migration_scan",
            description: "Whether memory from before the per-user layout is left to move.",
            inputs: vec![],
            outputs: out("{needed: bool, shared: bool}"),
        }),
        "migration_start" => Some(ControllerSchema {
            namespace: "memory",
            function: "migration_start",
            description: "Move memory from before the per-user layout now. takeover: true agrees to take a legacy tree other accounts on this machine may share.",
            inputs: vec![opt(
                "takeover",
                TypeSchema::Bool,
                "Consent to take a shared legacy tree.",
            )],
            outputs: out("{state: MigrationState, running: bool, interrupted: bool}"),
        }),
        "migration_status" => Some(ControllerSchema {
            namespace: "memory",
            function: "migration_status",
            description: "Progress of the move into the per-user layout.",
            inputs: vec![],
            outputs: out("{state: MigrationState, running: bool, interrupted: bool}"),
        }),
        "migration_retry" => Some(ControllerSchema {
            namespace: "memory",
            function: "migration_retry",
            description: "Put the items the move could not store back in line for the next run.",
            inputs: vec![],
            outputs: out("MigrationState"),
        }),
        _ => None,
    }
}
