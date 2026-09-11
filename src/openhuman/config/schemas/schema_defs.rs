use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::helpers::{json_output, optional_bool, optional_json, optional_string};

#[path = "schemas_schema_part_01.rs"]
mod schemas_schema_part_01;
#[path = "schemas_schema_part_02.rs"]
mod schemas_schema_part_02;

pub fn schemas(function: &str) -> ControllerSchema {
    if let Some(schema) = schemas_schema_part_01::lookup(function) {
        return schema;
    }
    if let Some(schema) = schemas_schema_part_02::lookup(function) {
        return schema;
    }
    ControllerSchema {
        namespace: "config",
        function: "unknown",
        description: "Unknown config controller function.",
        inputs: vec![],
        outputs: vec![FieldSchema {
            name: "error",
            ty: TypeSchema::String,
            comment: "Lookup error details.",
            required: true,
        }],
    }
}
