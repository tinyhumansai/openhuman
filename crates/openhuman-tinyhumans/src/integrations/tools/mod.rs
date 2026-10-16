use super::client::IntegrationClient;

mod google_places;
mod stock_prices;

pub use google_places::{GooglePlacesDetailsTool, GooglePlacesSearchTool};
pub use stock_prices::{
    StockCommodityTool, StockCryptoSeriesTool, StockExchangeRateTool, StockOptionsTool,
    StockQuoteTool,
};

/// The Google Places and stock-data tools over a client built from `config`,
/// or nothing when there is no usable backend credential. Each family is
/// gated by its own `[integrations.*]` switch.
///
/// Web search providers (Exa, Gemini, TinyFish, ...) register through the
/// TinySearch module, so `[search] enabled = false` suppresses them too;
/// `integrations.parallel` is parsed for old config files and no longer does
/// anything.
pub fn build_integration_tools(
    root_config: &openhuman_embed::__host::config::Config,
) -> Vec<Box<dyn tinytools::Tool>> {
    use std::sync::Arc;

    let mut tools: Vec<Box<dyn tinytools::Tool>> = Vec::new();
    let Some(client) = super::build_client(root_config) else {
        log::debug!("[integrations] build_client returned None; integration tools not registered");
        return tools;
    };
    log::debug!("[integrations] client built successfully");
    if root_config.integrations.google_places.is_active() {
        tools.push(Box::new(GooglePlacesSearchTool::new(Arc::clone(&client))));
        tools.push(Box::new(GooglePlacesDetailsTool::new(Arc::clone(&client))));
        log::debug!("[integrations] registered google_places tools");
    } else {
        log::debug!("[integrations] google_places disabled; skipping");
    }
    if root_config.integrations.stock_prices.is_active() {
        tools.push(Box::new(StockQuoteTool::new(Arc::clone(&client))));
        tools.push(Box::new(StockExchangeRateTool::new(Arc::clone(&client))));
        tools.push(Box::new(StockOptionsTool::new(Arc::clone(&client))));
        tools.push(Box::new(StockCryptoSeriesTool::new(Arc::clone(&client))));
        tools.push(Box::new(StockCommodityTool::new(Arc::clone(&client))));
        log::debug!("[integrations] registered stock_prices tools");
    } else {
        log::debug!("[integrations] stock_prices disabled; skipping");
    }
    tools
}

#[cfg(test)]
#[path = "families_tests.rs"]
mod families_tests;
