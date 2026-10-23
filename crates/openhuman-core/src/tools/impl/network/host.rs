//! Host wiring for the network tools that live in `tinytools_std::network`.
//!
//! The tools take their limits' fallbacks, their HTML engine and their 402
//! handler from the host. This file is where OpenHuman supplies them: the
//! `[http_request]` defaults, TinyJuice for page extraction, and (with the
//! `web3` feature) the x402 payment flow.

use crate::core::runtime::DispatchCtx;
use std::sync::Arc;

#[cfg(feature = "web3")]
use tinytools::{PermissionLevel, Tool, ToolResult};
use tinytools_std::network::{
    AsyncHtmlExtractor, CurlTool, HttpLimits, HttpRequestTool, NetGate, WebFetchTool,
};

use super::gate::RoutedGate;
use crate::config::HttpRequestConfig;
use crate::security::approval::ApprovalChatContext;
use crate::security::SecurityPolicy;
use crate::tools::per_call::PerCallTool;
use {tinyjuice_bus::tools::summary_focus_property, tinyjuice_bus::tools::SUMMARY_FOCUS_ARG};

/// The fallbacks for a `0` limit, from the `[http_request]` schema defaults, so
/// the tools, the schema and the 5→6 migration share one source.
fn http_limits() -> HttpLimits {
    let defaults = HttpRequestConfig::default();
    HttpLimits {
        max_response_size: defaults.max_response_size,
        timeout_secs: defaults.timeout_secs,
    }
}

/// HTML detection and extraction execute in the lazily loaded TinyJuice module.
#[derive(Debug)]
struct TinyJuiceHtml {
    ctx: DispatchCtx,
}

#[async_trait::async_trait]
impl AsyncHtmlExtractor for TinyJuiceHtml {
    async fn looks_like_html(&self, body: &str) -> anyhow::Result<bool> {
        let kind = crate::inference::tokenjuice::detect(
            body.to_string(),
            tinyjuice_bus::types::ContentHint::default(),
        )
        .await
        .map_err(anyhow::Error::msg)?;
        Ok(kind == "html")
    }

    async fn to_markdown(&self, html: &str) -> anyhow::Result<String> {
        crate::inference::tokenjuice::extract_html(&self.ctx, html.to_string())
            .await
            .map_err(anyhow::Error::msg)
    }
}

/// `http_request`, with the x402 402-retry wired in when `web3` is compiled.
///
/// For a caller that is not in a chat turn (a flow node, a direct dispatch): its
/// disclosures and x402 quotes carry no chat routing. A chat turn registers
/// [`routed_http_request_tool`] instead.
pub fn http_request_tool(
    ctx: &DispatchCtx,
    gate: Arc<dyn NetGate>,
    allowed_domains: Vec<String>,
    max_response_size: usize,
    timeout_secs: u64,
) -> HttpRequestTool {
    http_request_tool_for(
        ctx,
        gate,
        None,
        allowed_domains,
        max_response_size,
        timeout_secs,
    )
}

/// [`http_request_tool`] whose x402 quotes are owned by `chat`.
fn http_request_tool_for(
    ctx: &DispatchCtx,
    gate: Arc<dyn NetGate>,
    chat: Option<&ApprovalChatContext>,
    allowed_domains: Vec<String>,
    max_response_size: usize,
    timeout_secs: u64,
) -> HttpRequestTool {
    let tool = HttpRequestTool::new(
        gate,
        allowed_domains,
        max_response_size,
        timeout_secs,
        http_limits(),
    );
    #[cfg(feature = "web3")]
    let tool = tool.with_payment_hook(Arc::new(x402::X402PaymentHook::new(ctx, chat)));
    #[cfg(not(feature = "web3"))]
    let _ = (ctx, chat);
    tool
}

/// `http_request` for chat turns: built for each call around that call's chat,
/// so its egress disclosures and x402 quotes reach (and belong to) that chat.
pub fn routed_http_request_tool(
    policy: Arc<SecurityPolicy>,
    allowed_domains: Vec<String>,
    max_response_size: usize,
    timeout_secs: u64,
) -> PerCallTool<HttpRequestTool> {
    PerCallTool::new(move |chat, ctx, args| {
        http_request_tool_for(
            ctx,
            Arc::new(RoutedGate::new(Arc::clone(&policy), chat, args)),
            chat,
            allowed_domains.clone(),
            max_response_size,
            timeout_secs,
        )
    })
}

/// Dedicated x402 agent entry point backed by the same host HTTP policy and
/// the module payment hook used by `http_request`, built for each call around
/// that call's chat.
#[cfg(feature = "web3")]
pub(crate) fn x402_request_tool(
    policy: Arc<SecurityPolicy>,
    allowed_domains: Vec<String>,
    max_response_size: usize,
    timeout_secs: u64,
) -> PerCallTool<impl Tool> {
    PerCallTool::new(move |chat, ctx, args| {
        let gate: Arc<dyn NetGate> = Arc::new(RoutedGate::new(Arc::clone(&policy), chat, args));
        X402RequestTool {
            gate: Arc::clone(&gate),
            inner: http_request_tool_for(
                ctx,
                gate,
                chat,
                allowed_domains.clone(),
                max_response_size,
                timeout_secs,
            ),
        }
    })
}

#[cfg(feature = "web3")]
#[derive(Debug)]
struct X402RequestTool {
    gate: Arc<dyn NetGate>,
    inner: HttpRequestTool,
}

#[cfg(feature = "web3")]
#[async_trait::async_trait]
impl Tool for X402RequestTool {
    fn name(&self) -> &'static str {
        tinywallet_bus::tools::x402_request_tool_declaration().name
    }

    fn description(&self) -> &'static str {
        tinywallet_bus::tools::x402_request_tool_declaration().description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        tinywallet_bus::tools::x402_request_tool_declaration()
            .parameters
            .clone()
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn external_effect_with_args(&self, _args: &serde_json::Value) -> bool {
        self.gate.network_needs_approval()
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.inner.execute(args).await
    }
}

/// `curl` for chat turns: built for each call around that call's chat, so its
/// egress disclosure reaches that chat.
pub fn routed_curl_tool(
    policy: Arc<SecurityPolicy>,
    allowed_domains: Vec<String>,
    action_dir: std::path::PathBuf,
    dest_subdir: String,
    max_download_bytes: u64,
    timeout_secs: u64,
) -> PerCallTool<CurlTool> {
    PerCallTool::new(move |chat, _ctx, args| {
        CurlTool::new(
            Arc::new(RoutedGate::new(Arc::clone(&policy), chat, args)),
            allowed_domains.clone(),
            action_dir.clone(),
            dest_subdir.clone(),
            max_download_bytes,
            timeout_secs,
        )
    })
}

/// `web_fetch` for chat turns: built for each call around that call's chat, so
/// its egress disclosure reaches that chat.
pub fn routed_web_fetch_tool(
    policy: Arc<SecurityPolicy>,
    allowed_domains: Vec<String>,
    max_bytes: Option<usize>,
    timeout_secs: Option<u64>,
) -> PerCallTool<WebFetchTool> {
    PerCallTool::new(move |chat, ctx, args| {
        web_fetch_tool(
            ctx,
            Arc::new(RoutedGate::new(Arc::clone(&policy), chat, args)),
            allowed_domains.clone(),
            max_bytes,
            timeout_secs,
        )
    })
}

/// `web_fetch`, converting HTML through TinyJuice and opting into TinyJuice's
/// optional `summary_focus` argument.
pub fn web_fetch_tool(
    ctx: &DispatchCtx,
    gate: Arc<dyn NetGate>,
    allowed_domains: Vec<String>,
    max_bytes: Option<usize>,
    timeout_secs: Option<u64>,
) -> WebFetchTool {
    WebFetchTool::new_async(
        gate,
        allowed_domains,
        max_bytes,
        timeout_secs,
        http_limits(),
        Arc::new(TinyJuiceHtml { ctx: ctx.clone() }),
    )
    // Opts into a caller-steered summary; see `tokenjuice::focus`.
    .with_schema_property(SUMMARY_FOCUS_ARG, summary_focus_property())
}

#[cfg(feature = "web3")]
mod x402 {
    //! x402 machine payment for `http_request`'s `402 Payment Required` retry.

    use async_trait::async_trait;
    use tinytools_std::network::{PaymentAttempt, PaymentHook};

    use super::{ApprovalChatContext, DispatchCtx};
    use crate::modules::x402_service;

    /// `Err("COMPONENT_DISABLED:wallet")` when `[components] wallet` is off:
    /// the hook declines before any quote, signing or module load. The 402
    /// response then surfaces to the caller unpaid.
    pub(super) fn declined_when_wallet_off(config: &crate::config::Config) -> Result<(), String> {
        if config.components.wallet {
            return Ok(());
        }
        log::debug!("[tool.http_request] x402 payment declined: component 'wallet' disabled");
        Err(format!(
            "{}:wallet",
            crate::core::runtime::components::COMPONENT_DISABLED
        ))
    }

    /// The 402 handler for one `http_request` call. It owns the call's chat
    /// routing: `PaymentHook::pay` takes no run context, so the quote owner is
    /// held here and the hook is built per call.
    #[derive(Debug)]
    pub(super) struct X402PaymentHook {
        ctx: DispatchCtx,
        chat: Option<ApprovalChatContext>,
    }

    impl X402PaymentHook {
        pub(super) fn new(ctx: &DispatchCtx, chat: Option<&ApprovalChatContext>) -> Self {
            Self {
                ctx: ctx.clone(),
                chat: chat.cloned(),
            }
        }
    }

    #[async_trait]
    impl PaymentHook for X402PaymentHook {
        async fn pay(
            &self,
            url: &str,
            response_headers: &reqwest::header::HeaderMap,
        ) -> Result<PaymentAttempt, String> {
            let challenge = response_headers
                .get("PAYMENT-REQUIRED")
                .or_else(|| response_headers.get("X-PAYMENT-REQUIRED"))
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    "402 response is missing a valid PAYMENT-REQUIRED header".to_string()
                })?;
            let config = self
                .ctx
                .load_config()
                .await
                .map_err(|error| format!("x402 payment failed: {error}"))?;
            declined_when_wallet_off(&config)?;
            let owner = self
                .chat
                .as_ref()
                .map(|context| tinywallet_bus::wire::QuoteOwner {
                    thread_id: context.thread_id.clone(),
                    client_id: context.client_id.clone(),
                });
            let (service, payment) = x402_service::prepare_payment(
                &self.ctx,
                &config,
                url.to_owned(),
                challenge.to_owned(),
                owner,
            )
            .await
            .map_err(|error| format!("x402 payment failed: {error}"))?;

            log::debug!(
                "[tool.http_request] retrying with x402 payment header amount={} asset={}",
                payment.amount_atomic,
                payment.asset
            );

            let payment_id = payment.payment_id;
            let config = config.clone();
            Ok(PaymentAttempt {
                headers: vec![("PAYMENT-SIGNATURE".to_string(), payment.payment_header)],
                settle: Box::new(move |outcome| {
                    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                        runtime.spawn(async move {
                            if let Err(error) = x402_service::record_payment(
                                &config,
                                service,
                                payment_id,
                                outcome.status,
                                outcome.payment_response,
                            )
                            .await
                            {
                                log::warn!(
                                    "[tool.http_request] x402 settlement recording failed: {error}"
                                );
                            }
                        });
                    } else {
                        log::warn!(
                            "[tool.http_request] x402 settlement not recorded: runtime unavailable"
                        );
                    }
                }),
            })
        }
    }
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
