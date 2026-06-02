use crate::openhuman::config::Config;
use crate::openhuman::search::registry::SearchToolParams;
use crate::openhuman::tools::Tool;

pub(crate) fn build(_: &Config, _: SearchToolParams) -> Vec<Box<dyn Tool>> {
    tracing::debug!("[search] disabled — no search tools registered");
    Vec::new()
}
