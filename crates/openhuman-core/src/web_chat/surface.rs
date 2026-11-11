//! The host's adapter between a web-chat turn and TinyChannels' web projection
//! module. Canonical output crosses TinyBus; the host retains event delivery,
//! tenant routing, and web-chat-specific lifecycle policy.

use std::sync::Arc;

use sha2::Digest as _;
use tinychannels_bus::module_progressive::ChannelOutputEvent;
use tinychannels_bus::module_web_output::{
    BeginWebTurn, EndWebTurn, ProjectWebOutput, WebTurnHandle,
};
use tinychannels_bus::web as bus_web;

use super::event_bus_channel::publish_web_channel_event_in;
use crate::config::Config;
use crate::core::runtime::DispatchCtx;

/// Addressing of one turn on the web wire.
#[derive(Clone, Debug)]
pub(crate) struct TurnIds {
    pub(crate) client_id: String,
    pub(crate) thread_id: String,
    pub(crate) request_id: String,
}

struct SurfaceState {
    module: Option<tinybus::module::ModuleClient>,
    handle: Option<WebTurnHandle>,
    closed: bool,
    cleanup_done: Arc<tokio::sync::Notify>,
    cleanup_runtime: Option<tokio::runtime::Handle>,
    request_id: String,
}

/// One module-owned projection turn plus its host broadcast destination.
#[derive(Clone)]
pub(crate) struct Surface {
    state: Arc<tokio::sync::Mutex<SurfaceState>>,
    ctx: DispatchCtx,
    scope_key: String,
    ids: TurnIds,
    sequenced: bool,
}

impl Surface {
    /// A surface for the turn `ids`, publishing under `ctx`. Every canonical
    /// projection goes through the compiled TinyChannels module.
    pub(crate) async fn new(ctx: &DispatchCtx, ids: TurnIds, config: Config) -> Self {
        #[cfg(all(test, feature = "channels", feature = "modules"))]
        let module = {
            let _ = config;
            crate::channels::module_fixture::client().await
        };
        #[cfg(not(all(test, feature = "channels", feature = "modules")))]
        let module = crate::modules::client(config);

        let tenant = super::ops::tenant_of(ctx);
        let scope_bytes =
            serde_json::to_vec(&(tenant, &ids.client_id, &ids.thread_id, &ids.request_id))
                .expect("web turn scope tuple is serializable");
        let scope_key = format!(
            "web-chat:{}",
            hex::encode(sha2::Sha256::digest(scope_bytes))
        );

        Self {
            state: Arc::new(tokio::sync::Mutex::new(SurfaceState {
                module: Some(module),
                handle: None,
                closed: false,
                cleanup_done: Arc::new(tokio::sync::Notify::new()),
                cleanup_runtime: Some(tokio::runtime::Handle::current()),
                request_id: ids.request_id.clone(),
            })),
            ctx: ctx.clone(),
            scope_key,
            ids,
            sequenced: true,
        }
    }

    /// Publish this surface's projections without the module's `seq`. A late
    /// event projected after the turn's own projection closed gets a fresh
    /// module handle whose sequence restarts, and clients drop an event whose
    /// `seq` is not above the last one they saw for the request.
    pub(crate) fn unsequenced(mut self) -> Self {
        self.sequenced = false;
        self
    }

    /// A host-authored terminal event does not need a module handle when the
    /// turn never started canonical progress.
    pub(crate) fn host_only(ctx: &DispatchCtx, ids: TurnIds) -> Self {
        let tenant = super::ops::tenant_of(ctx);
        let scope_bytes =
            serde_json::to_vec(&(tenant, &ids.client_id, &ids.thread_id, &ids.request_id))
                .expect("web turn scope tuple is serializable");
        let scope_key = format!(
            "web-chat:{}",
            hex::encode(sha2::Sha256::digest(scope_bytes))
        );
        Self {
            state: Arc::new(tokio::sync::Mutex::new(SurfaceState {
                module: None,
                handle: None,
                closed: false,
                cleanup_done: Arc::new(tokio::sync::Notify::new()),
                cleanup_runtime: None,
                request_id: ids.request_id.clone(),
            })),
            ctx: ctx.clone(),
            scope_key,
            ids,
            sequenced: true,
        }
    }

    async fn begin_if_needed(&self, state: &mut SurfaceState) -> Result<String, String> {
        if state.closed {
            return Err("web projection turn is already closed".to_owned());
        }
        if let Some(handle) = &state.handle {
            return Ok(handle.handle.clone());
        }
        let module = state
            .module
            .as_ref()
            .ok_or_else(|| "module client is unavailable for canonical projection".to_owned())?;
        let response: WebTurnHandle = module
            .call_confidential(
                "tinychannels",
                tinychannels_bus::methods::BEGIN_WEB_TURN,
                (BeginWebTurn {
                    scope_key: self.scope_key.clone(),
                    context: self.context(),
                },),
            )
            .await
            .map_err(|error| error.to_string())?;
        let handle = response.handle.clone();
        state.handle = Some(response);
        Ok(handle)
    }

    async fn end_if_needed(&self, state: &mut SurfaceState) {
        state.closed = true;
        let Some(handle) = state.handle.take() else {
            return;
        };
        let Some(module) = state.module.as_ref() else {
            log::warn!(
                "[web-channel] projection handle has no module client request_id={}",
                self.ids.request_id
            );
            return;
        };
        if let Err(error) = module
            .call_confidential::<()>(
                "tinychannels",
                tinychannels_bus::methods::END_WEB_TURN,
                (EndWebTurn {
                    handle: handle.handle,
                },),
            )
            .await
        {
            log::warn!(
                "[web-channel] projection turn cleanup failed request_id={}: {error}",
                self.ids.request_id
            );
        }
    }

    /// Close a canonical projection even when a turn ended without a terminal
    /// output event. Safe to call after the module already auto-closed it.
    pub(crate) async fn close(&self) {
        let mut state = self.state.lock().await;
        self.end_if_needed(&mut state).await;
    }

    fn publish(
        &self,
        mut event: bus_web::WebChannelEvent,
        interim_round: Option<u32>,
        projected: bool,
    ) {
        if projected {
            event.normalize_projected_wire_fields();
        }
        if let bus_web::WebChannelEvent::ChatInterim { round, .. } = &mut event {
            if round.is_none() {
                *round = interim_round.filter(|round| *round > 0);
            }
        }
        publish_web_channel_event_in(&self.ctx, event);
    }

    /// Project one canonical output event onto the web wire, in module-owned
    /// sequence order. Module errors fail closed; the host never reimplements
    /// the projection locally.
    pub(crate) async fn output(&self, event: ChannelOutputEvent) {
        self.project(event, None).await;
    }

    async fn project(&self, event: ChannelOutputEvent, interim_round: Option<u32>) {
        let mut state = self.state.lock().await;
        if state.closed {
            log::debug!(
                "[web-channel] dropping output after terminal request_id={}",
                self.ids.request_id
            );
            return;
        }
        let handle = match self.begin_if_needed(&mut state).await {
            Ok(handle) => handle,
            Err(error) => {
                log::warn!(
                    "[web-channel] projection turn could not begin request_id={}: {error}",
                    self.ids.request_id
                );
                return;
            }
        };
        let Some(module) = state.module.as_ref() else {
            log::warn!(
                "[web-channel] output event has no module client request_id={}",
                self.ids.request_id
            );
            return;
        };
        let projected: tinychannels_bus::module_web_output::WebOutputEvents = match module
            .call_confidential(
                "tinychannels",
                tinychannels_bus::methods::PROJECT_WEB_OUTPUT,
                (ProjectWebOutput { handle, event },),
            )
            .await
        {
            Ok(projected) => projected,
            Err(error) => {
                log::warn!(
                    "[web-channel] output event not projected request_id={}: {error}",
                    self.ids.request_id
                );
                return;
            }
        };
        let terminal = projected.events.iter().any(is_terminal);
        for mut event in projected.events {
            if !self.sequenced {
                event.context_mut().seq = None;
            }
            self.publish(event, interim_round, true);
        }
        if terminal {
            self.end_if_needed(&mut state).await;
        }
    }

    /// The leading narration of `round`, flushed as an interim bubble.
    pub(crate) async fn interim(&self, round: u32, text: String) {
        self.project(
            ChannelOutputEvent::Clarification { prompt: text },
            Some(round),
        )
        .await;
    }

    /// Publish a host-authored event unchanged (queue, goal, todo, or the
    /// enriched terminal event). It has no canonical projection form, but it
    /// still closes any module-owned projection state for the turn.
    pub(crate) async fn event(&self, event: bus_web::WebChannelEvent) {
        let terminal = matches!(
            &event,
            bus_web::WebChannelEvent::ChatDone { .. }
                | bus_web::WebChannelEvent::ChatError { .. }
                | bus_web::WebChannelEvent::ChatCancelled { .. }
        );
        let mut state = self.state.lock().await;
        self.publish(event, None, false);
        if terminal {
            self.end_if_needed(&mut state).await;
        }
    }

    /// The addressing context for an event of this turn.
    pub(crate) fn context(&self) -> bus_web::WebEventContext {
        bus_web::WebEventContext {
            client_id: self.ids.client_id.clone(),
            thread_id: self.ids.thread_id.clone(),
            request_id: self.ids.request_id.clone(),
            ..Default::default()
        }
    }
}

impl Drop for SurfaceState {
    fn drop(&mut self) {
        let Some(handle) = self.handle.take() else {
            self.cleanup_done.notify_one();
            return;
        };
        let Some(module) = self.module.as_ref().cloned() else {
            self.cleanup_done.notify_one();
            return;
        };
        let Some(runtime) = self.cleanup_runtime.as_ref() else {
            log::warn!(
                "[web-channel] dropped projection turn without a cleanup runtime request_id={}",
                self.request_id
            );
            return;
        };
        let runtime = runtime.clone();
        let cleanup_done = Arc::clone(&self.cleanup_done);
        let request_id = self.request_id.clone();
        runtime.spawn(async move {
            if let Err(error) = module
                .call_confidential::<()>(
                    "tinychannels",
                    tinychannels_bus::methods::END_WEB_TURN,
                    (EndWebTurn {
                        handle: handle.handle,
                    },),
                )
                .await
            {
                log::warn!(
                    "[web-channel] dropped projection turn cleanup failed request_id={request_id}: {error}"
                );
            }
            cleanup_done.notify_one();
        });
    }
}

fn is_terminal(event: &bus_web::WebChannelEvent) -> bool {
    matches!(
        event,
        bus_web::WebChannelEvent::ChatDone { .. }
            | bus_web::WebChannelEvent::ChatError { .. }
            | bus_web::WebChannelEvent::ChatCancelled { .. }
    )
}

#[cfg(test)]
#[path = "surface_tests.rs"]
mod tests;
