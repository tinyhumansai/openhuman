//! Redirect-link shortener for token-heavy URLs.
//!
//! Long tracking URLs (e.g. `trip.com/forward/...?bizData=...`) burn tokens
//! whenever they pass through a model. This domain encodes them to a short
//! `openhuman://link/<id>` form for inbound prompts, keeps the full URL in
//! a local SQLite store, and expands them back on outbound messages so the
//! user never sees the placeholder.

pub mod ops;
mod schemas;
mod store;
mod types;

pub use ops as rpc;
pub use ops::{expand_link, rewrite_inbound, rewrite_outbound, shorten_url};
pub use schemas::{
    all_controller_schemas as all_redirect_links_controller_schemas,
    all_registered_controllers as all_redirect_links_registered_controllers,
    schemas as redirect_links_schemas,
};
pub use types::{RedirectLink, RewriteReplacement, RewriteResult};
