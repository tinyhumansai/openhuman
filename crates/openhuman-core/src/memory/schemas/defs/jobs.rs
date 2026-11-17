use super::common::{opt, out};
use crate::core::{ControllerSchema, TypeSchema};

pub(super) fn schema(function: &str) -> Option<ControllerSchema> {
    match function {
        "jobs_list" => Some(ControllerSchema {
            namespace: "memory",
            function: "jobs_list",
            description: "Memory's background job queue (belief builds, deferred ingests) and its latest runs.",
            inputs: vec![],
            outputs: out("{pending: QueuedJob[], history: JobRun[]}"),
        }),
        "jobs_run" => Some(ControllerSchema {
            namespace: "memory",
            function: "jobs_run",
            description: "Run queued background jobs now: one by id, or every pending one.",
            inputs: vec![opt(
                "id",
                TypeSchema::String,
                "One job; every pending job when omitted.",
            )],
            outputs: out("{runs: JobRun[]}"),
        }),
        _ => None,
    }
}
