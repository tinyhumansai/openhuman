//! Connected identity prompt helper.
//!
//! Kept in a dedicated sibling module so `mod.rs` remains mostly
//! export-focused while the runtime fetch logic lives in a small,
//! testable unit.

/// Render persisted provider identities (if available) as a compact
/// `## Connected Identities` section.
///
/// `load_connected_identities` reads through the bound memory driver now
/// (tinymemory v1.13.4 deleted the in-process engine's process-global client
/// this used to read synchronously — see
/// `integrations::composio::identity_store`'s module docs), which makes it
/// async. This function's ~7 call sites build a `PromptContext` struct
/// literal from a mix of sync and async functions, so rather than threading
/// `async`/`.await` through every one of them, this stays sync and uses the
/// same `block_in_place` + current-runtime-handle pattern
/// `session::builder::helpers::prefetch_tool_memory_rules_blocking` already
/// uses for the identical shape of problem. Best-effort: no runtime, a
/// single-threaded runtime, or a config-load/driver failure all render an
/// empty section rather than panicking or blocking a prompt on a memory read.
pub fn render_connected_identities() -> String {
    let identities = fetch_identities_blocking();
    crate::openhuman::integrations::composio::providers::render_connected_identities_section(
        &identities,
    )
}

fn fetch_identities_blocking() -> Vec<tinymemory_api::composio::ConnectedIdentity> {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return Vec::new();
    };
    if handle.runtime_flavor() != tokio::runtime::RuntimeFlavor::MultiThread {
        return Vec::new();
    }
    tokio::task::block_in_place(|| {
        handle.block_on(async move {
            let config = match crate::openhuman::config::rpc::load_config_with_timeout().await {
                Ok(config) => config,
                Err(error) => {
                    tracing::debug!(
                        %error,
                        "[prompts] render_connected_identities: config load failed"
                    );
                    return Vec::new();
                }
            };
            crate::openhuman::integrations::composio::identity_store::load_connected_identities(
                &config,
            )
            .await
            .unwrap_or_else(|error| {
                tracing::debug!(
                    %error,
                    "[prompts] render_connected_identities: load_connected_identities failed"
                );
                Vec::new()
            })
        })
    })
}
