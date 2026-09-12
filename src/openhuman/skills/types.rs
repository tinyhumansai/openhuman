//! Shared tool result types used by the tool and node runtime surfaces.
//!
//! The definitions live in [`tinytools`]; this module is the stable host import
//! path for the ~14 call sites that already name it, and the home of the one
//! conversion that is genuinely ours.

/// Serialized-size ceiling for a pass-through MCP content block before it
/// reaches a model prompt.
///
/// A block kind this build does not model is carried through as JSON rather
/// than dropped, and the generic payload of such a block can be a base64 image
/// or audio — megabytes. Above this ceiling the payload is elided, keeping the
/// block type but not the bytes.
const MAX_LLM_BLOCK_BYTES: usize = 64 * 1024;

pub use tinytools::{ToolContent, ToolResult};

/// Converts a rendered MCP result into this application's tool result.
///
/// The two shapes are the same by construction — the module's was derived from
/// this one when the client was extracted — so this is a mapping, not a
/// translation.
///
/// It is a free function rather than a `From` impl because both types are now
/// foreign to this crate: [`ToolResult`] belongs to `tinytools` and
/// `McpToolResult` to `tinymcp_bus`, which the orphan rule forbids us from
/// bridging with a trait impl. The reason the conversion is written **once**
/// has not changed with its shape: spelled out at each call site, it would be
/// as many chances to get the error flag the wrong way round.
pub fn tool_result_from_mcp(result: tinymcp_bus::McpToolResult) -> ToolResult {
    ToolResult {
        content: result
            .content
            .into_iter()
            .map(|block| match block {
                tinymcp_bus::McpToolContent::Text { text } => ToolContent::Text { text },
                tinymcp_bus::McpToolContent::Json { data } => ToolContent::Json { data },
                // The contract's block enum is `#[non_exhaustive]`. A kind this
                // build does not model is carried through as its JSON rather
                // than dropped: a caller can still read it, and dropping it
                // would lose content a server deliberately sent.
                other => ToolContent::Json {
                    data: elide_oversized_block(&other),
                },
            })
            .collect(),
        is_error: result.is_error,
        markdown_formatted: result.markdown_formatted,
    }
}

/// Serializes an unrecognized MCP content block, bounding what a model will
/// see.
///
/// The block is kept whole when it is small, so a future payload a server
/// deliberately sent still reaches the caller. Above [`MAX_LLM_BLOCK_BYTES`]
/// — the base64 image or audio case — the payload is replaced with an elided
/// marker while the block type is retained, so a prompt can still tell what
/// kind of content was dropped.
fn elide_oversized_block(block: &tinymcp_bus::McpToolContent) -> serde_json::Value {
    let value = serde_json::to_value(block).unwrap_or(serde_json::Value::Null);
    let serialized = serde_json::to_string(&value).unwrap_or_default();
    if serialized.len() <= MAX_LLM_BLOCK_BYTES {
        return value;
    }

    let kind = value
        .get("type")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    serde_json::json!({
        "type": kind,
        "data": format!("[{} bytes elided]", serialized.len()),
    })
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
