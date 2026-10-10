//! Host presets: the starting [`RuntimeBuilder`] for each kind of host.
//!
//! Each preset carries the domain/service/host-kind choices that host makes
//! today, so a host stops hand-assembling them. Every knob stays overridable
//! after the preset.
//!
//! | Preset | HostKind | DomainSet | ServiceSet | Workspace / config |
//! |---|---|---|---|---|
//! | [`library`](RuntimeBuilder::library) | `Library` | `embedded` (+`mcp`/`skills` when compiled) | none | ephemeral, [`Resolved`](ConfigSource::Resolved) |
//! | [`desktop`](RuntimeBuilder::desktop) | `TauriShell` | `full` | `desktop` | inherit, [`Discovered`](ConfigSource::Discovered) |
//! | [`cli`](RuntimeBuilder::cli) | `detect_standalone()` | `full` | `desktop` | inherit, `Discovered` |
//! | [`tui`](RuntimeBuilder::tui) | `detect_standalone()` | `full` | `none` | inherit, `Discovered` |
//!
//! Sources: the desktop and CLI rows are `openhuman-rpc`'s `run_server*`
//! shims (`server/shims.rs`), including their `OPENHUMAN_E2E` tool-group
//! switch; the TUI row is `openhuman-tui`'s `runner.rs`. All three boot the
//! core with no supplied config, which is why they use
//! [`ConfigSource::Discovered`]: handing the core a config would turn the
//! operator's install into a scoped embedder (no `active_user.toml`, a frozen
//! config). The token stays [`TokenSource::EnvOrFile`] — the desktop shell
//! passes its in-memory bearer with [`RuntimeBuilder::token`].

use openhuman_core::core::runtime::{DomainSet, ServiceSet, TokenSource};
use openhuman_core::core::types::HostKind;
use openhuman_core::tools::toolpacks::ToolGroups;

use super::builder::ConfigSource;
use super::RuntimeBuilder;
use crate::harness::Workspace;

impl RuntimeBuilder {
    /// The library preset — identical to [`RuntimeBuilder::new`]: an
    /// ephemeral workspace, [`HostKind::Library`], the embedded domains and
    /// only the harness init service.
    pub fn library() -> Self {
        Self::new()
    }

    /// The desktop shell preset: [`HostKind::TauriShell`], every domain,
    /// every background service, the operator's workspace discovered by the
    /// core. Pair with [`token`](Self::token)`(TokenSource::Fixed(..))` and
    /// [`listen`](Self::listen) as the shell does.
    pub fn desktop() -> Self {
        Self::operator_host(HostKind::TauriShell, ServiceSet::desktop()).with_e2e_tool_groups()
    }

    /// The standalone CLI server preset (`openhuman-core run` / `serve`):
    /// [`HostKind::detect_standalone`], every domain, every background
    /// service. For `--headless-api`, follow with
    /// [`services`](Self::services)`(ServiceSet::headless_api())`; for
    /// `--jsonrpc-only`, clear `socketio`.
    pub fn cli() -> Self {
        Self::operator_host(HostKind::detect_standalone(), ServiceSet::desktop())
            .with_e2e_tool_groups()
    }

    /// The terminal UI preset: [`HostKind::detect_standalone`], every domain
    /// (`channel.web_chat` needs the channels family), no transport and no
    /// background services — the TUI drives the core in-process.
    pub fn tui() -> Self {
        Self::operator_host(HostKind::detect_standalone(), ServiceSet::none())
    }

    fn operator_host(host_kind: HostKind, services: ServiceSet) -> Self {
        log::debug!("[embed][preset] operator host host_kind={host_kind:?} services={services:?}");
        Self {
            workspace: Workspace::Inherit,
            config_source: ConfigSource::Discovered,
            host_kind,
            domains: Some(DomainSet::full()),
            services: Some(services),
            token: TokenSource::EnvOrFile,
            ..Self::new()
        }
    }

    /// The browser E2E harness scripts direct tool calls through its mock
    /// model, so a server core under `OPENHUMAN_E2E` advertises every tool
    /// group; production keeps the fail-closed withheld default. Mirrors
    /// `openhuman-rpc`'s `apply_e2e_tool_groups`.
    fn with_e2e_tool_groups(mut self) -> Self {
        if std::env::var_os("OPENHUMAN_E2E").is_some() {
            log::debug!("[embed][preset] OPENHUMAN_E2E set: advertising every tool group");
            self.tool_groups = Some(ToolGroups::advertised());
        }
        self
    }
}

/// Domain families a library runtime registers by default: the embedded set
/// plus whichever of `mcp` / `skills` this build compiles in, because agents
/// can only narrow what the runtime registered.
pub(crate) fn default_domains() -> DomainSet {
    #[allow(unused_mut)]
    let mut domains = DomainSet::embedded();
    #[cfg(feature = "mcp")]
    {
        domains.mcp = true;
    }
    #[cfg(feature = "skills")]
    {
        domains.skills = true;
    }
    domains
}

/// Background services a library runtime runs by default: none.
pub(crate) fn default_services() -> ServiceSet {
    ServiceSet::none()
}

#[cfg(test)]
#[path = "presets_tests.rs"]
mod tests;
