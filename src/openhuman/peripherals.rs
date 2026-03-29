//! Hardware peripheral tools — board drivers were removed; config is accepted for compatibility.
//!
//! Returns an empty tool list so the agent loop and config parsing keep working.

use crate::openhuman::config::PeripheralsConfig;
use crate::openhuman::tools::Tool;

/// Previously connected serial/GPIO boards and exposed hardware tools. Always empty now.
pub async fn create_peripheral_tools(
    _config: &PeripheralsConfig,
) -> anyhow::Result<Vec<Box<dyn Tool>>> {
    Ok(Vec::new())
}
