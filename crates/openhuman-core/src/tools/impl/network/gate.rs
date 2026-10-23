//! `SecurityPolicy` as the network tools' [`NetGate`].
//!
//! A mechanical mapping: every method is one call the tools used to make on
//! the policy, the privacy-mode egress gate or the runtime proxy config
//! directly, with the same argument and the same answer. No policy is decided
//! here; the adapter only names the questions.

use tinytools_std::network::{Destination, DestinationFuture, NetGate};

use crate::config::PrivacyMode;
use crate::security::egress::assess::{assess_payload, Assessment};
use crate::security::egress::{
    emit_external_transfer, local_only_tool_block_blocking, DataKind, EgressDescriptor,
    IdentificationRisk,
};
use crate::security::{GateDecision, SecurityPolicy};
use tinysecurity_bus::CommandClass;

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
    /// What the call's own arguments (URL, headers, body) were assessed to
    /// carry, when the live privacy mode needs the answer (Sensitive).
    payload: Option<Assessment>,
}

impl RoutedGate {
    /// A gate for the call with arguments `args`. Under Sensitive the arguments
    /// are the payload that leaves the device, so the module assesses them once
    /// here and every descriptor this gate builds carries the answer; under any
    /// other mode nothing is sent to the module.
    pub(crate) fn new(
        policy: std::sync::Arc<SecurityPolicy>,
        chat: Option<&crate::security::approval::ApprovalChatContext>,
        args: &serde_json::Value,
    ) -> Self {
        let payload = (!args.is_null()
            && crate::security::live_policy::current_privacy_mode() == PrivacyMode::Sensitive)
            .then(|| assess_payload(&args.to_string()));
        Self {
            policy,
            chat: chat.cloned(),
            payload,
        }
    }

    /// `descriptor` with this call's payload assessment, if there is one.
    fn assessed(&self, descriptor: EgressDescriptor) -> EgressDescriptor {
        match &self.payload {
            Some(a) if a.level != IdentificationRisk::Unknown => {
                descriptor.with_risk(a.level, a.categories.clone())
            }
            _ => descriptor,
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
        local_only_tool_block_blocking(&self.assessed(EgressDescriptor::network_fetch(host)))
    }

    fn disclose(&self, host: &str, has_body: bool, has_headers: bool) {
        emit_external_transfer(
            self.chat.as_ref(),
            self.assessed(network_egress_descriptor(host, has_body, has_headers)),
        );
    }

    fn authorize_destination<'a>(
        &'a self,
        destination: &'a Destination<'a>,
    ) -> DestinationFuture<'a> {
        NetGate::authorize_destination(&*self.policy, destination)
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

/// The module's authorization for a tool destination, in the gate's terms.
///
/// Every destination is judged by the module, and a module that is absent,
/// untrusted, faulted or slow is a refusal carrying the tool-facing message.
async fn module_destination(
    policy: &SecurityPolicy,
    destination: &Destination<'_>,
) -> Option<Result<Vec<std::net::SocketAddr>, String>> {
    #[cfg(feature = "security-module")]
    {
        use crate::modules::security::egress::{authorize_tool_destination, ToolDestination};
        Some(
            authorize_tool_destination(
                &policy.workspace_dir,
                &ToolDestination {
                    url: destination.url,
                    method: destination.method,
                    headers: destination.headers,
                    body: destination.body,
                },
            )
            .await,
        )
    }
    #[cfg(not(feature = "security-module"))]
    {
        let _ = (policy, destination);
        Some(Err(format!(
            "{}TinySecurity egress module is unavailable",
            tinytools::status::POLICY_BLOCKED_MARKER
        )))
    }
}

/// The module's tier gate for a network-class action, or `None` when it cannot
/// answer.
fn network_gate(policy: &SecurityPolicy) -> Option<GateDecision> {
    match policy.command_class_gate_blocking(CommandClass::Network) {
        Ok(gate) => Some(gate),
        Err(refusal) => {
            log::warn!("[network][gate] tier gate unavailable; refusing: {refusal:?}");
            None
        }
    }
}

impl NetGate for SecurityPolicy {
    fn can_act(&self) -> bool {
        // The module's gate for network-class actions: a tier that blocks them
        // (read-only), or a module that cannot answer, refuses the call.
        self.tier_allows_acting_blocking()
            && network_gate(self).is_some_and(|gate| gate != GateDecision::Block)
    }

    fn is_rate_limited(&self) -> bool {
        // The hourly budget is spent once per call by the tool gate before the
        // tool runs; counting again here would charge two actions for one call.
        false
    }

    fn record_action(&self) -> bool {
        // See `is_rate_limited`: the call's action is already reserved.
        true
    }

    fn network_needs_approval(&self) -> bool {
        // An unanswered question is a prompt, never a silent allow; `can_act`
        // refuses the call itself when the module cannot answer.
        network_gate(self).is_none_or(|gate| gate == GateDecision::Prompt)
    }

    fn local_only_block(&self, host: &str) -> Option<String> {
        local_only_tool_block_blocking(&EgressDescriptor::network_fetch(host))
    }

    fn authorize_destination<'a>(
        &'a self,
        destination: &'a Destination<'a>,
    ) -> DestinationFuture<'a> {
        Box::pin(module_destination(self, destination))
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
