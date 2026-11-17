//! Public session accessors and mutators split by host responsibility.

use super::super::types::{OpenHumanSessionHost, SessionHostBuilder};
use std::collections::HashSet;
use std::sync::Arc;
use tinytools::{Tool, ToolSpec};
use tinytools_agent::dialect::TranscriptEntry;

mod registry;
mod runtime;
mod session;
mod tool_visibility;
mod turn_options;
