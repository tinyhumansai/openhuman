use crate::core::{FieldSchema, TypeSchema};

fn field(name: &'static str, ty: TypeSchema, comment: &'static str, required: bool) -> FieldSchema {
    FieldSchema {
        name,
        ty: if required {
            ty
        } else {
            TypeSchema::Option(Box::new(ty))
        },
        comment,
        required,
    }
}

pub(super) fn opt(name: &'static str, ty: TypeSchema, comment: &'static str) -> FieldSchema {
    field(name, ty, comment, false)
}

pub(super) fn req(name: &'static str, ty: TypeSchema, comment: &'static str) -> FieldSchema {
    field(name, ty, comment, true)
}

pub(super) fn out(comment: &'static str) -> Vec<FieldSchema> {
    vec![req("result", TypeSchema::Json, comment)]
}

pub(super) fn limit() -> FieldSchema {
    opt(
        "limit",
        TypeSchema::BoundedU64 { min: 1, max: 100 },
        "Most results (default 10).",
    )
}

pub(super) fn filter() -> FieldSchema {
    opt(
        "filter",
        TypeSchema::Json,
        "MetaFilter: metadata fields, kinds, sources, tags_any, observed_after/before, and reach ({at: namespace, inherit, descendants}) to read only some memory nodes.",
    )
}

pub(super) fn reach() -> FieldSchema {
    opt(
        "reach",
        TypeSchema::Json,
        "Reach {at: namespace, inherit, descendants}: only items in these memory nodes count; every node when omitted.",
    )
}

pub(super) fn path() -> FieldSchema {
    opt(
        "path",
        TypeSchema::Json,
        "Explorer path: [{facet, value}], each step narrowing the items (see memory_explore).",
    )
}

pub(super) fn cursor() -> FieldSchema {
    opt(
        "cursor",
        TypeSchema::String,
        "Engine cursor from a previous page.",
    )
}
