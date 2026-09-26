//! `tinychannels-bus` controller schemas → core [`ControllerSchema`]s.
//!
//! Always compiled (a dependency-free carve-out like `traits` / `cli`): the
//! core's channel controllers use it behind the `channels` gate, and
//! `openhuman-tinyhumans` uses it for the managed-bot link controllers it
//! serves (`channels.telegram_login_*`, `channels.discord_link_*`), whose
//! schemas the contract defines.

use tinychannels_bus::controllers::{
    channel_controller_schema, ChannelControllerField, ChannelControllerFieldType,
    ChannelControllerSchema,
};

use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

/// The contract's schema for channel controller `function`, converted.
pub fn contract_controller_schema(function: &str) -> ControllerSchema {
    from_channel_controller_schema(channel_controller_schema(function))
}

/// Convert one contract schema.
pub fn from_channel_controller_schema(schema: ChannelControllerSchema) -> ControllerSchema {
    ControllerSchema {
        namespace: schema.namespace,
        function: schema.function,
        description: schema.description,
        inputs: schema
            .inputs
            .into_iter()
            .map(from_channel_controller_field)
            .collect(),
        outputs: schema
            .outputs
            .into_iter()
            .map(from_channel_controller_field)
            .collect(),
    }
}

fn from_channel_controller_field(field: ChannelControllerField) -> FieldSchema {
    FieldSchema {
        name: field.name,
        ty: from_channel_controller_field_type(field.ty),
        comment: field.comment,
        required: field.required,
    }
}

fn from_channel_controller_field_type(ty: ChannelControllerFieldType) -> TypeSchema {
    match ty {
        ChannelControllerFieldType::Bool => TypeSchema::Bool,
        ChannelControllerFieldType::I64 => TypeSchema::I64,
        ChannelControllerFieldType::U64 => TypeSchema::U64,
        ChannelControllerFieldType::F64 => TypeSchema::F64,
        ChannelControllerFieldType::String => TypeSchema::String,
        ChannelControllerFieldType::Json => TypeSchema::Json,
        ChannelControllerFieldType::Option(inner) => {
            TypeSchema::Option(Box::new(from_channel_controller_field_type(*inner)))
        }
    }
}

#[cfg(test)]
#[path = "contract_schema_tests.rs"]
mod tests;
