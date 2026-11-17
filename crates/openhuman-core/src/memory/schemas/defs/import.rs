use super::common::{out, req};
use crate::core::{ControllerSchema, TypeSchema};

pub(super) fn schema(function: &str) -> Option<ControllerSchema> {
    match function {
        "import_scan" => Some(ControllerSchema {
            namespace: "memory",
            function: "import_scan",
            description: "Look for a v1 memory store in this workspace.",
            inputs: vec![],
            outputs: out("{found, counts?: {documents, conversations, learnings}}"),
        }),
        "import_start" => Some(ControllerSchema {
            namespace: "memory",
            function: "import_start",
            description: "Import the v1 store into the selected engine. Uploads local data; requires consent: true.",
            inputs: vec![req("consent", TypeSchema::Bool, "Must be true.")],
            outputs: out("{state: ImportState}"),
        }),
        "import_status" => Some(ControllerSchema {
            namespace: "memory",
            function: "import_status",
            description: "Progress of the v1 import.",
            inputs: vec![],
            outputs: out("{state: ImportState}"),
        }),
        "import_retry_failed" => Some(ControllerSchema {
            namespace: "memory",
            function: "import_retry_failed",
            description: "Store again the items a finished v1 import skipped because the engine refused them.",
            inputs: vec![],
            outputs: out("{state: ImportState}"),
        }),
        _ => None,
    }
}
