//! Host-platform services: process lifecycle, self-update, diagnostics and the
//! local transport surfaces the desktop shell talks to.
//!
//! Kernel, never gated — a build whose only driver is an external backend still
//! has to start, stay healthy, report cost, and be reachable over Socket.IO.

pub mod about_app;
pub mod connectivity;
pub mod cost;
pub mod doctor;
pub mod health;
pub mod proc_metrics;
pub mod service;
pub mod socket;
pub mod startup;
pub mod update;
