use super::common::{limit, opt, out, req};
use crate::core::{ControllerSchema, TypeSchema};

pub(super) fn schema(function: &str) -> Option<ControllerSchema> {
    match function {
        "brain_sources" => Some(ControllerSchema {
            namespace: "memory",
            function: "brain_sources",
            description: "The brain's sources (pdf, markdown, notion, github, web, …) and how many documents each holds.",
            inputs: vec![],
            outputs: out("{root, sources: {source, documents}[], unfiled}"),
        }),
        "brain_search" => Some(ControllerSchema {
            namespace: "memory",
            function: "brain_search",
            description: "Search the brain's documents, all sources or one.",
            inputs: vec![
                req("query", TypeSchema::String, "What to search for."),
                opt("source", TypeSchema::String, "One source's documents only."),
                limit(),
            ],
            outputs: out("{hits: Hit[]}"),
        }),
        "brain_ingest" => Some(ControllerSchema {
            namespace: "memory",
            function: "brain_ingest",
            description: "File a document in the brain: a local file (converted, its source picked from its format) or text. Queues the source's belief build.",
            inputs: vec![
                opt("path", TypeSchema::String, "A local file to read."),
                opt("text", TypeSchema::String, "Text to file."),
                opt(
                    "source",
                    TypeSchema::String,
                    "The source to file under (default: from the format, markdown for text).",
                ),
                opt("title", TypeSchema::String, "A title."),
            ],
            outputs: out("{id, source, replayed}"),
        }),
        "brain_forget" => Some(ControllerSchema {
            namespace: "memory",
            function: "brain_forget",
            description: "Forget every document of one brain source.",
            inputs: vec![req("source", TypeSchema::String, "The source.")],
            outputs: out("{forgotten: number}"),
        }),
        _ => None,
    }
}
