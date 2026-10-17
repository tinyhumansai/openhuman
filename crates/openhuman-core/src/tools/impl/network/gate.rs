//! `SecurityPolicy` as the network tools' [`NetGate`].
//!
//! A mechanical mapping: every method is one call the tools used to make on
//! the policy, the privacy-mode egress gate or the runtime proxy config
//! directly, with the same argument and the same answer. No policy is decided
//! here; the adapter only names the questions.

use tinytools_std::network::NetGate;

use crate::security::egress::{
    emit_external_transfer, local_only_tool_block, DataKind, EgressDescriptor,
};
use crate::security::{GateDecision, SecurityPolicy};
use tinybox_bus::CommandClass;

/// Build the egress descriptor for an outbound HTTP request (privacy epic S2,
/// #4436). The host is the destination; a request body carries tool arguments
/// and any custom headers (e.g. an `Authorization` token) are metadata that
/// also leaves the device — so a header-only call is not under-reported as
/// URL-only (codex P2, PR #4812). Pure over its inputs so it is unit-testable
/// off the network path.
pub(super) fn network_egress_descriptor(
    host: &str,
    has_body: bool,
    has_headers: bool,
) -> EgressDescriptor {
    let mut desc = EgressDescriptor::network_fetch(host);
    if has_body {
        desc = desc.with_data_kind(DataKind::ToolArguments);
    }
    if has_headers {
        desc = desc.with_data_kind(DataKind::Metadata);
    }
    desc
}

/// `SecurityPolicy` as the network tools' [`NetGate`] for one turn: every answer is
/// the policy's, and the disclosure additionally reaches the turn's chat.
///
/// `NetGate::disclose` takes no run context (the trait lives in `tinytools_std`),
/// so the chat the disclosure is routed to is held by the gate itself, which the
/// per-call tool wrapper ([`crate::tools::per_call::PerCallTool`]) builds for each
/// call.
#[derive(Debug)]
pub(crate) struct RoutedGate {
    policy: std::sync::Arc<SecurityPolicy>,
    chat: Option<crate::security::approval::ApprovalChatContext>,
}

impl RoutedGate {
    pub(crate) fn new(
        policy: std::sync::Arc<SecurityPolicy>,
        chat: Option<&crate::security::approval::ApprovalChatContext>,
    ) -> Self {
        Self {
            policy,
            chat: chat.cloned(),
        }
    }
}

impl NetGate for RoutedGate {
    fn can_act(&self) -> bool {
        NetGate::can_act(&*self.policy)
    }

    fn is_rate_limited(&self) -> bool {
        NetGate::is_rate_limited(&*self.policy)
    }

    fn record_action(&self) -> bool {
        NetGate::record_action(&*self.policy)
    }

    fn network_needs_approval(&self) -> bool {
        NetGate::network_needs_approval(&*self.policy)
    }

    fn local_only_block(&self, host: &str) -> Option<String> {
        NetGate::local_only_block(&*self.policy, host)
    }

    fn disclose(&self, host: &str, has_body: bool, has_headers: bool) {
        emit_external_transfer(
            self.chat.as_ref(),
            network_egress_descriptor(host, has_body, has_headers),
        );
    }

    fn prepare_client(
        &self,
        service: &str,
        builder: reqwest::ClientBuilder,
    ) -> reqwest::ClientBuilder {
        NetGate::prepare_client(&*self.policy, service, builder)
    }

    fn timeout_client(
        &self,
        service: &str,
        timeout_secs: u64,
        connect_timeout_secs: u64,
    ) -> reqwest::Client {
        NetGate::timeout_client(&*self.policy, service, timeout_secs, connect_timeout_secs)
    }
}

impl NetGate for SecurityPolicy {
    fn can_act(&self) -> bool {
        SecurityPolicy::can_act(self)
    }

    fn is_rate_limited(&self) -> bool {
        SecurityPolicy::is_rate_limited(self)
    }

    fn record_action(&self) -> bool {
        SecurityPolicy::record_action(self)
    }

    fn network_needs_approval(&self) -> bool {
        self.gate_decision(CommandClass::Network) == GateDecision::Prompt
    }

    fn local_only_block(&self, host: &str) -> Option<String> {
        local_only_tool_block(&EgressDescriptor::network_fetch(host))
    }

    fn disclose(&self, host: &str, has_body: bool, has_headers: bool) {
        // A caller that holds only the policy (a flow node, a direct dispatch) is
        // not in a chat turn; the tools a chat turn runs are built around a
        // `RoutedGate` instead.
        emit_external_transfer(None, network_egress_descriptor(host, has_body, has_headers));
    }

    fn prepare_client(
        &self,
        service: &str,
        builder: reqwest::ClientBuilder,
    ) -> reqwest::ClientBuilder {
        crate::config::apply_runtime_proxy_to_builder(builder, service)
    }

    fn timeout_client(
        &self,
        service: &str,
        timeout_secs: u64,
        connect_timeout_secs: u64,
    ) -> reqwest::Client {
        crate::config::build_runtime_proxy_client_with_timeouts(
            service,
            timeout_secs,
            connect_timeout_secs,
        )
    }
}

#[cfg(test)]
#[path = "gate_tests.rs"]
mod tests;
