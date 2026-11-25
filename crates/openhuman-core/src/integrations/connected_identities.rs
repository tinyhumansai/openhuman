//! Connected identity prompt helper.
//!
//! Kept in a dedicated sibling module so `mod.rs` remains mostly
//! export-focused while the runtime fetch logic lives in a small,
//! testable unit.

/// Render persisted provider identities (if available) as a compact
/// `## Connected Identities` section.
///
/// Prompt hooks are synchronous, but loading persisted identities is async.
/// Its I/O runs on a dedicated runtime: using the caller's runtime while
/// `block_in_place` waits can exhaust that runtime's blocking pool and prevent
/// the identity read itself from starting. The explicit dispatch context is
/// retained across the runtime hop. Outside a multi-thread runtime this remains
/// a best-effort empty section. Async turn preparation should use
/// [`render_connected_identities_async`] instead of blocking.
pub fn render_connected_identities(ctx: &crate::core::runtime::DispatchCtx) -> String {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return String::new();
    };
    if handle.runtime_flavor() != tokio::runtime::RuntimeFlavor::MultiThread {
        return String::new();
    }
    render_blocking(render_section(ctx))
}

/// Render connected identities from an async caller without blocking a Tokio
/// worker. Turn preparation should use this path while composing a new prompt.
pub async fn render_connected_identities_async(ctx: &crate::core::runtime::DispatchCtx) -> String {
    render_section(ctx).await
}

async fn render_section(ctx: &crate::core::runtime::DispatchCtx) -> String {
    let config = match ctx.load_config().await {
        Ok(config) => config,
        Err(error) => {
            tracing::debug!(%error, "[prompts] render_connected_identities: config load failed");
            return String::new();
        }
    };
    let identities = match crate::integrations::composio::identity_store::load_connected_identities(
        ctx, &config,
    )
    .await
    {
        Ok(identities) => identities,
        Err(error) => {
            tracing::debug!(
                %error,
                "[prompts] render_connected_identities: load_connected_identities failed"
            );
            return String::new();
        }
    };
    // The section is rendered by the connector module (`NormalizeIdentities`).
    crate::integrations::composio::contract::render_connected_identities_section(
        &config,
        &identities,
    )
    .await
    .unwrap_or_else(|error| {
        tracing::debug!(%error, "[prompts] render_connected_identities: render failed");
        String::new()
    })
}

fn render_blocking(render: impl std::future::Future<Output = String>) -> String {
    static RUNTIME: std::sync::OnceLock<Result<tokio::runtime::Runtime, std::io::Error>> =
        std::sync::OnceLock::new();
    let runtime = RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("identity-prompt")
            .enable_all()
            .build()
    });
    let Ok(runtime) = runtime else {
        tracing::debug!("[prompts] render_connected_identities: runtime unavailable");
        return String::new();
    };
    tokio::task::block_in_place(|| runtime.block_on(render))
}

#[cfg(test)]
#[path = "connected_identities_tests.rs"]
mod tests;
