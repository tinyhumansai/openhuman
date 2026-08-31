//! Tavily Search + Extract integration — direct API (BYOK, not backend-proxied).
//!
//! **Scope**: Agent + CLI/RPC.
//!
//! **Endpoints**: `POST https://api.tavily.com/search`,
//! `POST https://api.tavily.com/extract`.
//!
//! **Auth**: `Authorization: Bearer <api key>`.
//!
//! When the user selects `tavily` as their search engine and has saved their own
//! Tavily API key, every call in this family goes straight from the desktop
//! client to `api.tavily.com` — the OpenHuman managed backend is never
//! involved. The managed (`engine = "managed"`) path is untouched by this module.

use crate::openhuman::tools::traits::{Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

const DEFAULT_API_URL: &str = "https://api.tavily.com";
const SEARCH_EXCERPT_MAX_CHARS: usize = 500;
const SEARCH_RAW_CONTENT_MAX_CHARS: usize = 8_000;
const MAX_QUERY_IMAGES: usize = 10;
const MAX_IMAGES_PER_RESULT: usize = 3;
const IMAGE_DESCRIPTION_MAX_CHARS: usize = 300;

#[cfg(test)]
#[path = "tavily_tests.rs"]
mod tests;

// Layout gate (scripts/ci/check-openhuman-rust-layout.mjs, 750-line limit):
// the implementation continues in included part files, mirroring parallel.rs.
include!("tavily_part_01.rs");
include!("tavily_part_02.rs");
