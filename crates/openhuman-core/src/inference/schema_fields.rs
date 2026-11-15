//! Shared constructors for the core's RPC schema fields.

use crate::core::{FieldSchema, TypeSchema};

pub(crate) fn optional_u64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
        comment,
        required: false,
    }
}
