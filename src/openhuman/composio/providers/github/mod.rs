//! GitHub Composio toolkit — curated tool catalog only.
//!
//! There is no native [`super::ComposioProvider`] implementation for
//! GitHub yet (no profile fetch / sync). The curated catalog here is
//! still consulted by [`super::catalog_for_toolkit`] so the meta-tool
//! layer applies the same whitelist + scope filtering it does for
//! Gmail and Notion.

pub mod tools;

pub use tools::GITHUB_CURATED;
