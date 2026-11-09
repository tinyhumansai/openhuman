use super::common::{cursor, filter, limit, opt, out, path, reach, req};
use crate::core::{ControllerSchema, TypeSchema};

pub(super) fn schema(function: &str) -> Option<ControllerSchema> {
    match function {
        "recall" => Some(ControllerSchema {
            namespace: "memory",
            function: "recall",
            description: "Ask memory a question; returns an answer with citations.",
            inputs: vec![
                req("question", TypeSchema::String, "The question."),
                filter(),
                limit(),
            ],
            outputs: out("{answer, citations: Citation[], model?}"),
        }),
        "fetch" => Some(ControllerSchema {
            namespace: "memory",
            function: "fetch",
            description: "Raw retrieval over stored items, filtered by metadata.",
            inputs: vec![
                req("query", TypeSchema::String, "What to search for."),
                opt(
                    "mode",
                    TypeSchema::String,
                    "keyword | vector | hybrid; limited to the engine's fetch_modes.",
                ),
                filter(),
                limit(),
                cursor(),
            ],
            outputs: out("{hits: Hit[], next_cursor?}"),
        }),
        "learn" => Some(ControllerSchema {
            namespace: "memory",
            function: "learn",
            description: "Store a learning.",
            inputs: vec![
                req("text", TypeSchema::String, "The learning."),
                opt(
                    "kind",
                    TypeSchema::String,
                    "preference | fact | procedure | correction | other (default fact).",
                ),
                opt(
                    "confidence",
                    TypeSchema::F64,
                    "Confidence in 0..=1 (default 0.8).",
                ),
                opt("meta", TypeSchema::Json, "MemoryMeta to attach."),
            ],
            outputs: out("{id}"),
        }),
        "forget" => Some(ControllerSchema {
            namespace: "memory",
            function: "forget",
            description: "Remove items by id.",
            inputs: vec![
                req(
                    "ids",
                    TypeSchema::Array(Box::new(TypeSchema::String)),
                    "Item ids.",
                ),
                reach(),
            ],
            outputs: out("{forgotten: number}"),
        }),
        "erase_all" => Some(ControllerSchema {
            namespace: "memory",
            function: "erase_all",
            description: "Erase all memory the bound engine holds, for good. On the hosted engine this erases the account's entire hosted memory.",
            inputs: vec![req(
                "confirm",
                TypeSchema::Bool,
                "Must be true: nothing erased comes back.",
            )],
            outputs: out("{erased_scopes: number}"),
        }),
        "items_list" => Some(ControllerSchema {
            namespace: "memory",
            function: "items_list",
            description: "Page through stored items, newest first.",
            inputs: vec![
                filter(),
                limit(),
                cursor(),
                path(),
                opt(
                    "preview",
                    TypeSchema::Bool,
                    "Snippet listing: a conversation or chunked document may carry only its start; read it whole with items_get.",
                ),
            ],
            outputs: out("{items: Hit[], next_cursor?}"),
        }),
        "explore" => Some(ControllerSchema {
            namespace: "memory",
            function: "explore",
            description: "Count stored items per value of one facet (kind, source, source_id, workspace, folder, file_path, language, repo, url, thread, agent, tool_call, tag), under an explorer path.",
            inputs: vec![
                req("facet", TypeSchema::String, "The facet to group by."),
                path(),
                filter(),
                opt(
                    "limit",
                    TypeSchema::BoundedU64 { min: 1, max: 500 },
                    "Most buckets, largest first (default 50).",
                ),
                opt(
                    "scan_limit",
                    TypeSchema::BoundedU64 {
                        min: 1,
                        max: 50_000,
                    },
                    "Most items a listing-based engine reads (default 5000).",
                ),
            ],
            outputs: out(
                "{facet, buckets: {value, count}[], total, missing, more_buckets, truncated}",
            ),
        }),
        "items_get" => Some(ControllerSchema {
            namespace: "memory",
            function: "items_get",
            description: "Read stored items whole by id, in the order asked; unknown ids are left out.",
            inputs: vec![
                req(
                    "ids",
                    TypeSchema::Array(Box::new(TypeSchema::String)),
                    "Item ids (1 to 200).",
                ),
                reach(),
            ],
            outputs: out("{items: Hit[]}"),
        }),
        _ => None,
    }
}
