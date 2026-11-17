//! Everything host-specific about reaching the TinyHumans backend: where it
//! is, how requests are attributed, and who they are attributed to.
//!
//! The core knows none of this. It asks the installed
//! [`BackendTransport`](openhuman_embed::BackendTransport) —
//! [`SdkBackendTransport`](crate::SdkBackendTransport) — which answers from
//! these modules:
//!
//! - [`url`] — base-URL resolution: the production/staging defaults,
//!   `BACKEND_URL` / `VITE_BACKEND_URL` overrides, and the guard that keeps an
//!   `api_url` pointed at an inference endpoint away from control-plane calls.
//! - [`headers`] — the attribution headers (`x-core-version`,
//!   `x-tauri-version`, `x-sdk-name`) and per-profile `reqwest` clients.
//! - [`product`] — the process-wide product identity (`x-sdk-name`), set once
//!   at startup by [`crate::install`] or an embedding product.

pub mod headers;
pub mod product;
pub mod url;

pub use product::{
    product_identity, product_identity_header, product_identity_headers, set_product_identity,
    ProductIdentity, DEFAULT_PRODUCT_IDENTITY, PRODUCT_IDENTITY_HEADER,
};
pub use url::{effective_api_url, effective_backend_api_url, DEFAULT_API_BASE_URL};
