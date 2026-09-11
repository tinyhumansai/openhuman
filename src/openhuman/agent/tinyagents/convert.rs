//! Tool-schema conversion retained at the OpenHuman/TinyAgents tool seam.
//!
//! Durable message conversion lives in `agent::message_convert`, beside the
//! OpenHuman transcript record it adapts. This module remains until WP-4
//! decides the host tool-trait boundary.

use tinyinference::tool::ToolSchema;

use crate::openhuman::tools::ToolSpec;

pub(crate) fn spec_to_schema(spec: &ToolSpec) -> ToolSchema {
    ToolSchema::new(
        spec.name.clone(),
        spec.description.clone(),
        spec.parameters.clone(),
    )
}

#[cfg(test)]
#[path = "convert_tests.rs"]
mod tests;
