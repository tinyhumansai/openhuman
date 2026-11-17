//! Builder seams after a real `build()`: a controller extension is invokable
//! through the runtime, a live policy is applied once the core has booted, and
//! the documented drop behaviour holds (hooks removed; the registry, which has
//! no removal, keeps the extension).
//!
//! One test, one runtime: a runtime claims a process-wide slot.

mod common;

use openhuman_core::core::runtime::DispatchCtx;
use std::sync::Arc;

use common::{offline_config, runtime, stub_backend};
use openhuman_core::core::all::{ControllerFuture, RegisteredController};
use openhuman_core::core::ControllerSchema;
use openhuman_embed::seams::{ControllerExtension, DomainGroup, LearningSink, SecurityPolicy};
use openhuman_embed::{schema_for_rpc_method, Runtime, Workspace};
use serde_json::{json, Map, Value};

const METHOD: &str = "openhuman.embed_seam_probe_ping";
const POLICY_MARKER: u32 = 4242;

fn ping(_ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { Ok(json!({ "pong": true })) })
}

fn extension() -> ControllerExtension {
    ControllerExtension {
        group: DomainGroup::Platform,
        controllers: vec![RegisteredController {
            schema: ControllerSchema {
                namespace: "embed_seam_probe",
                function: "ping",
                description: "Seam probe.",
                inputs: vec![],
                outputs: vec![],
            },
            handler: ping,
        }],
        namespaces: &[("embed_seam_probe", "Embed seam probe.")],
    }
}

struct ProbeHook;

#[async_trait::async_trait]
impl LearningSink for ProbeHook {
    async fn on_turn_complete(
        &self,
        _ctx: &openhuman_embed::seams::TurnSummary,
    ) -> tinyagents_harness::error::Result<()> {
        Ok(())
    }
}

fn hook_installed() -> bool {
    !openhuman_core::agent::hooks::embedder_post_turn_hooks().is_empty()
}

#[test]
fn seams_are_live_after_build_and_unwound_on_drop() {
    // The runtime must not pick up a storage backend from the operator's shell.
    std::env::remove_var("OPENHUMAN_STORAGE_URL");
    let tokio = runtime();
    tokio.block_on(async {
        tokio::spawn(async {
            assert!(
                schema_for_rpc_method(METHOD).is_none(),
                "not registered yet"
            );
            let backend = stub_backend().await;

            let policy = SecurityPolicy {
                max_actions_per_hour: POLICY_MARKER,
                ..SecurityPolicy::default()
            };

            let runtime = Runtime::builder()
                .config(offline_config())
                .workspace(Workspace::Ephemeral)
                .backend_url(backend.uri())
                .controller_extension(extension())
                .post_turn_hook(Arc::new(ProbeHook))
                .live_policy(Arc::new(policy))
                .build()
                .await
                .expect("runtime builds");

            let reply = runtime
                .core_runtime()
                .invoke(METHOD, json!({}))
                .await
                .expect("the extension's method is invokable after build");
            assert_eq!(reply, json!({ "pong": true }));
            assert!(schema_for_rpc_method(METHOD).is_some());
            assert!(hook_installed(), "hook installed while the runtime lives");

            let live = openhuman_core::security::live_policy::current()
                .expect("a live policy is installed after build");
            assert_eq!(
                live.max_actions_per_hour, POLICY_MARKER,
                "the builder's policy replaced the one the core booted with"
            );

            drop(runtime);
            assert!(!hook_installed(), "hook removed when the runtime drops");
            assert!(
                schema_for_rpc_method(METHOD).is_some(),
                "the registry has no removal: the extension outlives the runtime"
            );
        })
        .await
        .expect("test task");
    });
}
