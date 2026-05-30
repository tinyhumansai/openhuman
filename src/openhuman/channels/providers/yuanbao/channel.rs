//! Channel facade for the Yuanbao provider.
//!
//! This module owns the OpenHuman [`Channel`] implementation and keeps
//! provider wiring out of `mod.rs`. Protocol decoding, transport, inbound
//! filtering, and outbound sending remain delegated to sibling modules.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{mpsc, watch, Mutex as TokioMutex};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::openhuman::channels::traits::{Channel, ChannelMessage, SendMessage};

use super::config::YuanbaoConfig;
use super::connection::{InboundEvent, YuanbaoConnection};
use super::ids::{shorten_account_id, shorten_reply_target};
use super::inbound::{InboundPipeline, PipelineOutcome, PipelineState};
use super::outbound::OutboundSender;
use super::proto::decode_push_msg;
use super::sign::SignManager;
use super::{splitter, types};

/// Reply Heartbeat keepalive interval. The yuanbao gateway expects the
/// bot to ping (`SendPrivateHeartbeat RUNNING`) at this cadence so the
/// "正在输入" indicator stays alive for long-running responses.
const REPLY_HEARTBEAT_INTERVAL_SECS: u64 = 2;

/// Hard ceiling on the in-memory shortened-recipient → original-recipient
/// map. Each entry is two short strings (~80 B), so 4096 distinct senders
/// give ~320 KB — plenty for any realistic chat load and small enough
/// that we can blow the whole map away when we hit the cap instead of
/// dragging in an LRU dependency. See `register_recipient_alias`.
const RECIPIENT_ALIAS_CAP: usize = 4096;

/// The yuanbao channel — owns one WebSocket and one inbound pipeline.
pub struct YuanbaoChannel {
    config: YuanbaoConfig,
    connection: Arc<YuanbaoConnection>,
    outbound: Arc<OutboundSender>,
    pipeline: Arc<InboundPipeline>,
    shutdown_tx: watch::Sender<bool>,
    /// Holds the inbound receiver between `new()` and the first `listen()` call.
    ///
    /// `Channel::listen` takes `&self`, so we can't move the receiver out of
    /// a field. Use a `Mutex<Option<…>>` so the first listener takes ownership
    /// and subsequent calls fail cleanly.
    inbound_rx: parking_lot::Mutex<Option<mpsc::UnboundedReceiver<InboundEvent>>>,
    /// Per-recipient Reply Heartbeat keepalive tasks (started on `start_typing`).
    heartbeat_tasks: TokioMutex<HashMap<String, JoinHandle<()>>>,
    /// Reverse lookup table from shortened recipient ids (the ones we
    /// emit on `ChannelMessage.sender` / `reply_target`) back to the
    /// original server-recognized ids that outbound `send_c2c_message`
    /// / `send_group_message` must use as `to_account` / `group_code`.
    ///
    /// Why this exists: yuanbao uids are ~64-char hashes, and
    /// `super::ids::shorten_account_id` rewrites them as
    /// `<prefix>_<sha256-16hex>` so the conversation store's per-thread
    /// JSONL filenames stay under filesystem `NAME_MAX`. Without this
    /// table the agent loop sends replies addressed to the shortened
    /// hash, which the yuanbao gateway silently drops because no such
    /// user exists. See `register_recipient_alias` / `resolve_recipient`.
    recipient_aliases: TokioMutex<HashMap<String, String>>,
}

impl YuanbaoChannel {
    /// Build a channel from a validated config. Returns an error if the
    /// config is missing required fields (so misconfiguration surfaces
    /// at startup, not on the first inbound message).
    pub fn new(mut config: YuanbaoConfig) -> anyhow::Result<Self> {
        config.apply_env_defaults();
        config.validate()?;
        let (shutdown_tx, _shutdown_rx) = watch::channel(false);
        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel::<InboundEvent>();

        // SignManager is only useful when we have an app_secret — without
        // it we'd never call the sign endpoint anyway.
        let sign_manager: Option<Arc<SignManager>> = if !config.app_secret.is_empty() {
            Some(SignManager::new(reqwest::Client::new()))
        } else {
            None
        };

        let connection = YuanbaoConnection::new(config.clone(), inbound_tx, sign_manager.clone());
        let outbound = Arc::new(OutboundSender::new(
            Arc::clone(&connection),
            sign_manager.clone(),
            config.app_key.clone(),
            config.bot_id.clone(),
        ));
        // PipelineState's `from_account` is used by the echo-guard stage to
        // drop self-sent messages. We feed it the static config value here
        // (which may be empty); the canonical server-issued bot_id only
        // becomes known after sign-token, so this is a known minor gap —
        // echo guard will simply not fire when bot_id isn't statically set.
        let pipeline_state = PipelineState::new(&config, config.bot_id.clone());
        let pipeline = Arc::new(InboundPipeline::new(pipeline_state));

        Ok(Self {
            config,
            connection,
            outbound,
            pipeline,
            shutdown_tx,
            inbound_rx: parking_lot::Mutex::new(Some(inbound_rx)),
            heartbeat_tasks: TokioMutex::new(HashMap::new()),
            recipient_aliases: TokioMutex::new(HashMap::new()),
        })
    }

    /// Record a `shortened → original` recipient mapping so the outbound
    /// side can recover the server-recognized id when the agent loop
    /// addresses a reply with the shortened sender / reply_target it
    /// received on `ChannelMessage`.
    ///
    /// No-op when the two are equal (uid is short enough to skip
    /// shortening, or this is the `g:` group-target case where the
    /// inner code is short). When the map crosses `RECIPIENT_ALIAS_CAP`
    /// we clear it — the next inbound message from each active sender
    /// re-populates the entry it needs, and stale entries from idle
    /// conversations are fine to lose.
    async fn register_recipient_alias(&self, shortened: &str, original: &str) {
        if shortened == original {
            return;
        }
        let mut m = self.recipient_aliases.lock().await;
        if m.len() >= RECIPIENT_ALIAS_CAP {
            warn!(
                "[yuanbao] recipient alias map hit cap ({}), clearing",
                RECIPIENT_ALIAS_CAP
            );
            m.clear();
        }
        m.insert(shortened.to_string(), original.to_string());
    }

    /// Look up the server-recognized recipient for a (possibly
    /// shortened) inbound id. Falls back to the input unchanged when
    /// nothing is registered — which keeps the previous behavior for
    /// recipients that don't go through `shorten_account_id` (short
    /// uids, group codes, `imessage`-style ids).
    async fn resolve_recipient(&self, recipient: &str) -> String {
        let m = self.recipient_aliases.lock().await;
        m.get(recipient)
            .cloned()
            .unwrap_or_else(|| recipient.to_string())
    }

    fn split_message(&self, text: &str) -> Vec<String> {
        splitter::split_markdown(text, self.config.max_message_length)
    }

    async fn start_heartbeat_task(&self, recipient: &str) {
        let mut tasks = self.heartbeat_tasks.lock().await;
        if tasks.contains_key(recipient) {
            return;
        }
        let outbound = Arc::clone(&self.outbound);
        let target = recipient.to_string();
        let handle = tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_secs(REPLY_HEARTBEAT_INTERVAL_SECS));
            interval.tick().await; // skip first tick (start_typing already sent RUNNING)
            loop {
                interval.tick().await;
                if let Err(e) = outbound.start_heartbeat(&target).await {
                    // Connection bouncing — bail out of this loop; the
                    // next start_typing call will spawn a new one.
                    warn!(
                        "[yuanbao] reply heartbeat send failed: {} — stopping loop",
                        e
                    );
                    return;
                }
            }
        });
        tasks.insert(recipient.to_string(), handle);
    }

    async fn stop_heartbeat_task(&self, recipient: &str) {
        let mut tasks = self.heartbeat_tasks.lock().await;
        if let Some(handle) = tasks.remove(recipient) {
            handle.abort();
        }
    }
}

#[async_trait]
impl Channel for YuanbaoChannel {
    fn name(&self) -> &str {
        "yuanbao"
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        let chunks = self.split_message(&message.content);
        let ref_msg_id = message.thread_ts.as_deref();
        let recipient = self.resolve_recipient(&message.recipient).await;
        for chunk in &chunks {
            self.outbound
                .send_text(&recipient, chunk, ref_msg_id)
                .await?;
        }
        Ok(())
    }

    fn supports_draft_updates(&self) -> bool {
        // Routes turns through the streaming code path even though Yuanbao
        // itself has no edit-message capability. We accept the UX cost (no
        // progressive rendering — the reply appears all at once in
        // `finalize_draft`) in exchange for streaming's tolerance of
        // malformed `usage` chunks; the non-streaming parser fails the
        // whole turn when an upstream LLM returns string-typed token counts.
        true
    }

    async fn send_draft(&self, message: &SendMessage) -> anyhow::Result<Option<String>> {
        // Marker id so dispatch spins up the progress consumer task;
        // nothing is sent to the user here. Real content goes out in
        // `finalize_draft`. See `supports_draft_updates` for rationale.
        Ok(Some(format!("yb-draft:{}", message.recipient)))
    }

    async fn update_draft(
        &self,
        _recipient: &str,
        _message_id: &str,
        _text: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn finalize_draft(
        &self,
        recipient: &str,
        _message_id: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> anyhow::Result<()> {
        let chunks = self.split_message(text);
        let recipient = self.resolve_recipient(recipient).await;
        for chunk in &chunks {
            self.outbound
                .send_text(&recipient, chunk, thread_ts)
                .await?;
        }
        Ok(())
    }

    async fn listen(&self, tx: mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        // Take the inbound receiver. A second listener would just exit early.
        let mut inbound_rx = match self.inbound_rx.lock().take() {
            Some(rx) => rx,
            None => {
                warn!("[yuanbao] listen() called twice — second call exits");
                return Ok(());
            }
        };

        let conn = Arc::clone(&self.connection);
        let shutdown_rx = self.shutdown_tx.subscribe();
        let mut conn_task = tokio::spawn(async move {
            conn.run(shutdown_rx).await;
        });

        info!("[yuanbao] channel listening — pipeline ready");
        let mut shutdown_rx2 = self.shutdown_tx.subscribe();
        loop {
            tokio::select! {
                _ = shutdown_rx2.changed() => {
                    info!("[yuanbao] listen loop received shutdown");
                    break;
                }
                event = inbound_rx.recv() => {
                    match event {
                        Some(InboundEvent::Push(frame)) => {
                            self.dispatch_push(frame, &tx).await;
                        }
                        Some(InboundEvent::Kickout(reason)) => {
                            warn!("[yuanbao] kickout: {} — stopping listen loop", reason);
                            break;
                        }
                        None => {
                            warn!("[yuanbao] inbound channel closed");
                            break;
                        }
                    }
                }
            }
        }

        let _ = self.shutdown_tx.send(true);
        // Give the connection task a brief window to run its own shutdown cleanup
        // (flush pending, update is_connected, etc.) before force-aborting.
        match tokio::time::timeout(std::time::Duration::from_secs(2), &mut conn_task).await {
            Ok(_) => {}
            Err(_) => conn_task.abort(),
        }
        Ok(())
    }

    async fn health_check(&self) -> bool {
        self.connection.is_connected()
    }

    async fn start_typing(&self, recipient: &str) -> anyhow::Result<()> {
        // Send RUNNING immediately, then spawn a 2s keepalive so the
        // indicator doesn't expire while we generate.
        let recipient = self.resolve_recipient(recipient).await;
        self.outbound.start_heartbeat(&recipient).await?;
        self.start_heartbeat_task(&recipient).await;
        Ok(())
    }

    async fn stop_typing(&self, recipient: &str) -> anyhow::Result<()> {
        let recipient = self.resolve_recipient(recipient).await;
        self.stop_heartbeat_task(&recipient).await;
        self.outbound.stop_heartbeat(&recipient).await?;
        Ok(())
    }

    fn supports_reactions(&self) -> bool {
        false
    }
}

impl YuanbaoChannel {
    async fn dispatch_push(&self, frame: types::ConnFrame, tx: &mpsc::Sender<ChannelMessage>) {
        // The Yuanbao gateway pushes inbound messages with `cmd_type=Push`;
        // the actual `cmd` word is decided server-side and varies (mirrors
        // hermes-agent `yuanbao.py::_handle_received_frame` which routes
        // purely on cmd_type). The connection layer has already filtered
        // out non-push frames before we get here, so every frame we see
        // should be a candidate for the inbound pipeline.
        if frame.data.is_empty() {
            tracing::trace!("[yuanbao] empty push body cmd={} — skipping", frame.cmd);
            return;
        }
        // Some push frames wrap the biz body in an extra
        // `PushMsg { cmd, module, msg_id, data }` envelope; others (e.g.
        // cmd="inbound_message", module="yuanbao_openclaw_proxy") put the
        // InboundMessagePush bytes directly in `ConnMsg.data` with the
        // ConnMsg.head already carrying cmd/module. Mirrors plugin
        // client.ts::onPush (l. 813): try PushMsg first, but only accept
        // it when it has a non-empty cmd or module; otherwise treat the
        // raw frame.data as the biz body.
        let unwrapped: Option<Vec<u8>> = match decode_push_msg(&frame.data) {
            Ok(p) if (!p.cmd.is_empty() || !p.module.is_empty()) && !p.data.is_empty() => {
                info!(
                    "[yuanbao] push envelope decoded: cmd={} module={} msg_id={} biz_len={}",
                    p.cmd,
                    p.module,
                    p.msg_id,
                    p.data.len()
                );
                Some(p.data)
            }
            _ => {
                info!(
                    "[yuanbao] push has no PushMsg envelope — treating ConnMsg.data as biz body (conn_cmd={} module={} len={})",
                    frame.cmd,
                    frame.module,
                    frame.data.len()
                );
                None
            }
        };
        let biz_body: &[u8] = unwrapped.as_deref().unwrap_or(&frame.data);
        let outcome = self.pipeline.process(biz_body).await;
        match outcome {
            PipelineOutcome::Dispatch(ctx) => {
                // Shorten ids at the channel boundary so the composite thread_id
                // derived downstream (channel:yuanbao_<sender>_<reply_target>)
                // stays under filesystem NAME_MAX once hex-encoded for the
                // per-thread JSONL filename. Yuanbao internals (echo guard,
                // access control, owner-command check) keep the original
                // `from_account` — see `super::ids` for the format and rationale.
                let original_from = ctx.msg.from_account.clone();
                let original_reply_target = ctx.source.reply_target();
                let short_sender = shorten_account_id(&original_from);
                let short_reply_target = shorten_reply_target(&original_reply_target);
                // Remember the original ids so the outbound side can
                // recover them when the agent loop addresses a reply
                // with the shortened values it sees here.
                self.register_recipient_alias(&short_sender, &original_from)
                    .await;
                self.register_recipient_alias(&short_reply_target, &original_reply_target)
                    .await;
                let msg = ChannelMessage {
                    id: ctx.msg.msg_id.clone(),
                    sender: short_sender,
                    reply_target: short_reply_target,
                    content: if ctx.text.is_empty() && !ctx.image_urls.is_empty() {
                        // Surface image URLs as content so downstream tools have something to work with.
                        ctx.image_urls.join("\n")
                    } else {
                        ctx.text.clone()
                    },
                    channel: "yuanbao".into(),
                    timestamp: ctx.msg.msg_time as u64,
                    thread_ts: None,
                };
                if tx.send(msg).await.is_err() {
                    warn!("[yuanbao] dispatch receiver gone — dropping message");
                }
            }
            PipelineOutcome::Filtered(reason) => {
                tracing::trace!("[yuanbao] filtered at {reason}");
            }
            PipelineOutcome::Failed(err) => {
                // Intentionally omit the raw biz payload — it can carry
                // user content / PII. The decoder error already encodes
                // the structural reason; only the length is safe to log.
                warn!(
                    "[yuanbao] pipeline error: {err} | biz_len={}",
                    biz_body.len()
                );
            }
        }
    }
}

impl Drop for YuanbaoChannel {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
    }
}

#[cfg(test)]
mod tests {
    use crate::openhuman::channels::traits::Channel;

    use super::*;

    fn good_cfg() -> YuanbaoConfig {
        let mut c = YuanbaoConfig::default();
        c.app_key = "ak".into();
        c.ws_domain = "wss://example".into();
        c.token = "tok".into();
        c
    }

    #[test]
    fn channel_construction_validates() {
        let ch = YuanbaoChannel::new(good_cfg()).unwrap();
        assert_eq!(ch.name(), "yuanbao");
    }

    #[test]
    fn invalid_config_rejected() {
        let mut c = YuanbaoConfig::default();
        c.app_key = "ak".into();
        // missing ws_domain
        assert!(YuanbaoChannel::new(c).is_err());
    }

    #[test]
    fn split_short_message_returns_one() {
        let ch = YuanbaoChannel::new(good_cfg()).unwrap();
        let chunks = ch.split_message("hello");
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn split_respects_newlines() {
        let mut c = good_cfg();
        c.max_message_length = 12;
        let ch = YuanbaoChannel::new(c).unwrap();
        let chunks = ch.split_message("line one\nline two\nline three");
        assert!(chunks.len() >= 2);
        // No chunk exceeds the limit.
        for chunk in &chunks {
            assert!(chunk.len() <= 12, "chunk too long: {chunk:?}");
        }
    }

    #[tokio::test]
    async fn resolve_recipient_returns_input_when_no_alias_registered() {
        let ch = YuanbaoChannel::new(good_cfg()).unwrap();
        assert_eq!(ch.resolve_recipient("short_uid").await, "short_uid");
    }

    #[tokio::test]
    async fn register_and_resolve_dm_alias_recovers_original_uid() {
        let ch = YuanbaoChannel::new(good_cfg()).unwrap();
        let original = "x".repeat(64);
        let shortened = shorten_account_id(&original);
        assert_ne!(shortened, original, "test premise: should actually shorten");
        ch.register_recipient_alias(&shortened, &original).await;
        assert_eq!(ch.resolve_recipient(&shortened).await, original);
    }

    #[tokio::test]
    async fn register_recipient_alias_is_noop_for_equal_pair() {
        let ch = YuanbaoChannel::new(good_cfg()).unwrap();
        // Short uid that wouldn't be shortened — caller still hands us
        // (s, s); we should silently skip and not eat a map slot.
        ch.register_recipient_alias("short", "short").await;
        let m = ch.recipient_aliases.lock().await;
        assert!(m.is_empty());
    }

    #[tokio::test]
    async fn resolve_recipient_preserves_group_prefix_via_alias() {
        let ch = YuanbaoChannel::new(good_cfg()).unwrap();
        let long_group_code = "g".repeat(64);
        let original = format!("g:{long_group_code}");
        let shortened = shorten_reply_target(&original);
        assert_ne!(shortened, original);
        assert!(shortened.starts_with("g:"));
        ch.register_recipient_alias(&shortened, &original).await;
        assert_eq!(ch.resolve_recipient(&shortened).await, original);
    }

    #[tokio::test]
    async fn alias_map_clears_when_cap_is_hit() {
        let ch = YuanbaoChannel::new(good_cfg()).unwrap();
        // Pre-fill up to the cap with distinct entries.
        for i in 0..RECIPIENT_ALIAS_CAP {
            ch.register_recipient_alias(&format!("s{i}"), &format!("o{i}"))
                .await;
        }
        assert_eq!(ch.recipient_aliases.lock().await.len(), RECIPIENT_ALIAS_CAP);
        // One more entry must trigger a clear, then insert the new entry.
        ch.register_recipient_alias("new_short", "new_original")
            .await;
        let m = ch.recipient_aliases.lock().await;
        assert_eq!(m.len(), 1);
        assert_eq!(m.get("new_short").map(String::as_str), Some("new_original"));
    }

    // ─── trivial trait methods ─────────────────────────────────────

    #[test]
    fn supports_draft_updates_is_true() {
        let ch = YuanbaoChannel::new(good_cfg()).unwrap();
        assert!(ch.supports_draft_updates());
    }

    #[test]
    fn supports_reactions_is_false() {
        let ch = YuanbaoChannel::new(good_cfg()).unwrap();
        assert!(!ch.supports_reactions());
    }

    #[tokio::test]
    async fn send_draft_returns_marker_id() {
        let ch = YuanbaoChannel::new(good_cfg()).unwrap();
        let msg = SendMessage::new("ignored", "user-42");
        let id = ch.send_draft(&msg).await.unwrap();
        assert_eq!(id.as_deref(), Some("yb-draft:user-42"));
    }

    #[tokio::test]
    async fn update_draft_is_a_noop_ok() {
        let ch = YuanbaoChannel::new(good_cfg()).unwrap();
        assert!(ch.update_draft("user-42", "any-id", "text").await.is_ok());
    }

    #[tokio::test]
    async fn health_check_is_false_when_socket_not_connected() {
        // Real connect requires a WebSocket; we only verify the
        // disconnected default here. The connected branch is exercised
        // by `connection::tests::set_state_connected_flips_is_connected_flag`.
        let ch = YuanbaoChannel::new(good_cfg()).unwrap();
        assert!(!ch.health_check().await);
    }

    // ─── dispatch_push branches ────────────────────────────────────

    fn make_push_frame(cmd: &str, data: Vec<u8>) -> types::ConnFrame {
        types::ConnFrame {
            cmd_type: super::super::proto_constants::cmd_type::PUSH,
            cmd: cmd.into(),
            module: "yuanbao_openclaw_proxy".into(),
            seq_no: 0,
            msg_id: String::new(),
            need_ack: false,
            status: 0,
            data,
        }
    }

    #[tokio::test]
    async fn dispatch_push_empty_body_is_skipped() {
        let ch = YuanbaoChannel::new(good_cfg()).unwrap();
        let (tx, mut rx) = mpsc::channel::<ChannelMessage>(4);
        let frame = make_push_frame("noop", Vec::new());
        ch.dispatch_push(frame, &tx).await;
        // No message should reach the sender.
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn dispatch_push_garbage_body_does_not_dispatch() {
        // Body is not a valid protobuf push *and* not valid JSON → Failed.
        // dispatch_push should log + swallow, not propagate panic.
        let ch = YuanbaoChannel::new(good_cfg()).unwrap();
        let (tx, mut rx) = mpsc::channel::<ChannelMessage>(4);
        let frame = make_push_frame("inbound_message", vec![0xFF, 0xFF, 0xFF, 0xFF]);
        ch.dispatch_push(frame, &tx).await;
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn dispatch_push_dm_text_reaches_listener() {
        // Build a minimal `InboundMessagePush` directly in ConnFrame.data
        // (no PushMsg envelope), with a single TIMTextElem so the pipeline
        // dispatches.
        use super::super::proto::{encode_msg_body_element, encode_varint};
        let elem = types::MsgBodyElement {
            msg_type: "TIMTextElem".into(),
            msg_content: types::MsgContent {
                text: Some("hello".into()),
                ..Default::default()
            },
        };
        let elem_bytes = encode_msg_body_element(&elem);

        // Hand-roll an InboundMessagePush so we don't depend on a helper:
        // field 2 = from_account, field 3 = to_account, field 12 = msg_id,
        // field 13 = repeated MsgBodyElement.
        let mut biz = Vec::new();
        let put_string = |fnum: u32, s: &str, b: &mut Vec<u8>| {
            encode_varint(((fnum as u64) << 3) | 2, b);
            encode_varint(s.len() as u64, b);
            b.extend_from_slice(s.as_bytes());
        };
        put_string(2, "alice", &mut biz);
        put_string(3, "bot1", &mut biz);
        put_string(12, "mid-x", &mut biz);
        encode_varint(((13u64) << 3) | 2, &mut biz);
        encode_varint(elem_bytes.len() as u64, &mut biz);
        biz.extend_from_slice(&elem_bytes);

        // Disable group_at_required and use open dm_access so the
        // pipeline passes all stages for this DM.
        let mut cfg = good_cfg();
        cfg.dm_access = "open".into();
        cfg.bot_id = "bot1".into();
        let ch = YuanbaoChannel::new(cfg).unwrap();

        let frame = make_push_frame("inbound_message", biz);
        let (tx, mut rx) = mpsc::channel::<ChannelMessage>(4);
        ch.dispatch_push(frame, &tx).await;
        let msg = rx.try_recv().expect("dispatch should produce one message");
        assert_eq!(msg.id, "mid-x");
        assert_eq!(msg.content, "hello");
        assert_eq!(msg.channel, "yuanbao");
    }

    #[tokio::test]
    async fn dispatch_push_filtered_by_dedup_does_not_double_dispatch() {
        use super::super::proto::{encode_msg_body_element, encode_varint};
        let elem = types::MsgBodyElement {
            msg_type: "TIMTextElem".into(),
            msg_content: types::MsgContent {
                text: Some("dup".into()),
                ..Default::default()
            },
        };
        let elem_bytes = encode_msg_body_element(&elem);
        let mut biz = Vec::new();
        let put_string = |fnum: u32, s: &str, b: &mut Vec<u8>| {
            encode_varint(((fnum as u64) << 3) | 2, b);
            encode_varint(s.len() as u64, b);
            b.extend_from_slice(s.as_bytes());
        };
        put_string(2, "alice", &mut biz);
        put_string(3, "bot1", &mut biz);
        put_string(12, "dup-id", &mut biz);
        encode_varint(((13u64) << 3) | 2, &mut biz);
        encode_varint(elem_bytes.len() as u64, &mut biz);
        biz.extend_from_slice(&elem_bytes);

        let mut cfg = good_cfg();
        cfg.dm_access = "open".into();
        cfg.bot_id = "bot1".into();
        let ch = YuanbaoChannel::new(cfg).unwrap();
        let (tx, mut rx) = mpsc::channel::<ChannelMessage>(4);
        ch.dispatch_push(make_push_frame("inbound_message", biz.clone()), &tx)
            .await;
        assert!(rx.try_recv().is_ok(), "first should dispatch");
        ch.dispatch_push(make_push_frame("inbound_message", biz), &tx)
            .await;
        assert!(rx.try_recv().is_err(), "second (same id) should dedup");
    }

    // ─── heartbeat task lifecycle ──────────────────────────────────

    #[tokio::test]
    async fn start_heartbeat_task_inserts_and_stop_removes() {
        let ch = YuanbaoChannel::new(good_cfg()).unwrap();
        ch.start_heartbeat_task("recipient-1").await;
        assert!(
            ch.heartbeat_tasks.lock().await.contains_key("recipient-1"),
            "should have spawned a task for recipient-1"
        );
        // Second start for same recipient is a no-op (does not double-spawn).
        ch.start_heartbeat_task("recipient-1").await;
        assert_eq!(ch.heartbeat_tasks.lock().await.len(), 1);

        ch.stop_heartbeat_task("recipient-1").await;
        assert!(ch.heartbeat_tasks.lock().await.is_empty());
        // Stopping a recipient with no task is also a no-op.
        ch.stop_heartbeat_task("never-started").await;
    }
}
