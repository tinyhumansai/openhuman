//! Axum-based HTTP gateway with proper HTTP/1.1 compliance, body limits, and timeouts.

mod client;
mod constants;
mod handlers;
mod models;
mod rate_limit;
mod server;
mod state;

#[cfg(test)]
mod tests;

pub use server::run_gateway;
