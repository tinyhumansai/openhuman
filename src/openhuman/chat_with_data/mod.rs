//! Chat-with-data and proactive insights domain.
//!
//! Natural-language querying over local/connected datasets with proactive
//! insight generation (anomaly detection, trend analysis, summaries).
//!
//! Log prefix: `[chat_with_data]`

pub mod anomaly;
pub mod db_connector;
pub mod engine;
mod rpc;
mod schemas;
pub mod sql_gen;
pub mod types;
pub mod webhooks;

pub use schemas::{
    all_controller_schemas as all_chat_with_data_controller_schemas,
    all_registered_controllers as all_chat_with_data_registered_controllers,
    schemas as chat_with_data_schemas,
};
pub use types::{DataSource, DatasetMeta, Insight, InsightType, QueryResult};
