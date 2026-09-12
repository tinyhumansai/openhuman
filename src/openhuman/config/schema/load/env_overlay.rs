use super::super::proxy::{
    normalize_no_proxy_list, normalize_proxy_url_option, normalize_service_list,
    parse_proxy_enabled, parse_proxy_scope, set_runtime_proxy_config, ProxyScope,
};
use super::super::{Config, UpdateRestartStrategy};
use super::dirs::MEMORY_SYNC_INTERVAL_SECS_ENV_VAR;
use super::env::parse_env_bool;
use std::path::PathBuf;

/// Classification of an `OPENHUMAN_SHELL_HIDE_WINDOW` env value. Split out from
/// the apply site (where the three cases differ only by log level) so the
/// empty-vs-unrecognized distinction is unit-testable without capturing tracing
/// output — a bare `VAR=` must classify as `Unset` (silent no-op), not
/// `Unrecognized` (which warns).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ShellHideWindowParse {
    /// Empty / whitespace-only — the var is present but has no value; treat as
    /// absent (no change, no warning).
    Unset,
    /// A recognized boolean value.
    Set(bool),
    /// A non-empty value that isn't a recognized boolean — warn and ignore.
    Unrecognized,
}

pub(super) fn classify_shell_hide_window(raw: &str) -> ShellHideWindowParse {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" => ShellHideWindowParse::Unset,
        "1" | "true" | "yes" | "on" => ShellHideWindowParse::Set(true),
        "0" | "false" | "no" | "off" => ShellHideWindowParse::Set(false),
        _ => ShellHideWindowParse::Unrecognized,
    }
}
include!("env_overlay_impl_01_part_01.rs");
include!("env_overlay_impl_01_part_02.rs");
