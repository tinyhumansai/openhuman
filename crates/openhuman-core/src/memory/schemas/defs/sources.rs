use super::common::{opt, out, req};
use crate::core::{ControllerSchema, TypeSchema};

pub(super) fn schema(function: &str) -> Option<ControllerSchema> {
    match function {
        "sources_list" => Some(ControllerSchema {
            namespace: "memory",
            function: "sources_list",
            description: "List document sources with their sync state.",
            inputs: vec![],
            outputs: out("{sources: Source[]}"),
        }),
        "sources_add" => Some(ControllerSchema {
            namespace: "memory",
            function: "sources_add",
            description: "Add a document source.",
            inputs: vec![
                req(
                    "kind",
                    TypeSchema::String,
                    "folder | file | link | github | rss.",
                ),
                req(
                    "target",
                    TypeSchema::String,
                    "Path, URL, owner/repo or feed URL.",
                ),
                opt(
                    "label",
                    TypeSchema::String,
                    "Display label (default: the target).",
                ),
                opt(
                    "schedule_mins",
                    TypeSchema::BoundedU64 {
                        min: 15,
                        max: u64::from(u32::MAX),
                    },
                    "Minutes between scheduled syncs; omit for on demand only.",
                ),
                opt(
                    "namespace",
                    TypeSchema::String,
                    "Layout root to file the documents under, e.g. `team:acme`; the configured root (shared by every agent) when omitted.",
                ),
            ],
            outputs: out("{source: Source}"),
        }),
        "sources_remove" => Some(ControllerSchema {
            namespace: "memory",
            function: "sources_remove",
            description: "Remove a document source.",
            inputs: vec![
                req("id", TypeSchema::String, "Source id."),
                opt(
                    "forget_items",
                    TypeSchema::Bool,
                    "Also forget everything it stored.",
                ),
            ],
            outputs: out("{removed: boolean}"),
        }),
        "sources_sync" => Some(ControllerSchema {
            namespace: "memory",
            function: "sources_sync",
            description: "Start syncing one source, or all of them.",
            inputs: vec![opt(
                "id",
                TypeSchema::String,
                "Source id; every source when omitted.",
            )],
            outputs: out("{started: string[]}"),
        }),
        _ => None,
    }
}
