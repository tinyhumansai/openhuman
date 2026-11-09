//! SDK-owned Socket.IO transport that implements the core's socket port.

use openhuman_embed::__host::platform::socket::TokenProvider;
use std::sync::Arc;

mod port;
pub(crate) use port::SdkSocketPort;

pub(crate) fn register() {
    openhuman_embed::__host::platform::socket::set_global_socket_manager(Arc::new(
        SdkSocketPort::default(),
    ));
}

#[cfg(test)]
fn sdk_token_provider(
    provider: TokenProvider,
) -> impl Fn() -> Option<String> + Send + Sync + 'static {
    sdk_token_provider_with_observer(provider, |_| {})
}

fn sdk_token_provider_with_observer(
    provider: TokenProvider,
    observer: impl Fn(String) + Send + Sync + 'static,
) -> impl Fn() -> Option<String> + Send + Sync + 'static {
    let observer = Arc::new(observer);
    move || {
        let provider = provider.clone();
        let observer = Arc::clone(&observer);
        let token = openhuman_embed::__host::storage::block_on_anyhow(async move {
            provider().await.map_err(anyhow::Error::msg)
        })
        .ok()?;
        if token.trim().is_empty() {
            return None;
        }
        observer(token.clone());
        Some(token)
    }
}
