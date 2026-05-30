//! Inbound message pipeline (17 stages).
//!
//! Mirrors `InboundPipeline` in hermes-agent `gateway/platforms/yuanbao.py`.
//! Each stage runs in order; any of them can short-circuit by
//! returning `Skip(reason)`. `Abort(_)` propagates an error.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::RwLock;
use tracing::{debug, trace};

use super::config::YuanbaoConfig;
use super::errors::YuanbaoError;
use super::proto::{decode_inbound_json, decode_inbound_push};
use super::proto_constants::*;
use super::types::*;

/// Shared per-channel state that survives across messages.
pub struct PipelineState {
    pub bot_id: String,
    pub bot_name: String,
    pub owner_id: String,
    pub dm_access: AccessPolicy,
    pub group_access: AccessPolicy,
    pub allowed_users: Vec<String>,
    pub allowed_groups: Vec<String>,
    pub group_at_required: bool,
    pub home_chat: RwLock<Option<String>>,
    pub dedup: RwLock<DedupCache>,
}

impl PipelineState {
    pub fn new(cfg: &YuanbaoConfig, bot_id: String) -> Arc<Self> {
        Arc::new(Self {
            bot_id,
            bot_name: cfg.bot_name.clone(),
            owner_id: cfg.owner_id.clone(),
            dm_access: AccessPolicy::parse(&cfg.dm_access),
            group_access: AccessPolicy::parse(&cfg.group_access),
            allowed_users: cfg.allowed_users.clone(),
            allowed_groups: cfg.allowed_groups.clone(),
            group_at_required: cfg.group_at_required,
            home_chat: RwLock::new(None),
            dedup: RwLock::new(DedupCache::new(DEDUP_CAPACITY, DEDUP_TTL_SECS)),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessPolicy {
    Open,
    Allowlist,
    Closed,
}

impl AccessPolicy {
    fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "open" => Self::Open,
            "closed" | "disabled" | "none" => Self::Closed,
            _ => Self::Allowlist,
        }
    }
}

/// Mutable context passed through every inbound stage.
#[derive(Debug, Clone)]
pub struct PipelineCtx {
    pub msg: InboundMessage,
    pub source: Source,
    pub text: String,
    pub image_urls: Vec<String>,
    pub is_at_bot: bool,
    pub is_owner_command: bool,
    pub kind: MessageKind,
}

/// Outcome of a single inbound stage invocation.
#[derive(Debug)]
pub enum MwResult {
    Continue,
    Skip(&'static str),
    Abort(YuanbaoError),
}

/// Final outcome of the whole pipeline.
#[derive(Debug)]
pub enum PipelineOutcome {
    Dispatch(PipelineCtx),
    Filtered(&'static str),
    Failed(YuanbaoError),
}

#[async_trait]
pub trait Middleware: Send + Sync {
    fn name(&self) -> &'static str;
    async fn process(&self, state: &PipelineState, ctx: &mut PipelineCtx) -> MwResult;
}

/// LRU-like dedup cache with TTL.
pub struct DedupCache {
    capacity: usize,
    ttl: Duration,
    order: VecDeque<(String, Instant)>,
    index: std::collections::HashSet<String>,
}

impl DedupCache {
    pub fn new(capacity: usize, ttl_secs: u64) -> Self {
        Self {
            capacity,
            ttl: Duration::from_secs(ttl_secs),
            order: VecDeque::with_capacity(capacity),
            index: std::collections::HashSet::with_capacity(capacity),
        }
    }

    /// Returns `true` if `id` has been seen within the TTL window. Inserts it otherwise.
    pub fn check_and_insert(&mut self, id: &str) -> bool {
        self.evict_expired();
        if self.index.contains(id) {
            return true;
        }
        if self.order.len() >= self.capacity {
            if let Some((old, _)) = self.order.pop_front() {
                self.index.remove(&old);
            }
        }
        self.order.push_back((id.to_string(), Instant::now()));
        self.index.insert(id.to_string());
        false
    }

    fn evict_expired(&mut self) {
        let now = Instant::now();
        while let Some((_, ts)) = self.order.front() {
            if now.duration_since(*ts) > self.ttl {
                if let Some((old, _)) = self.order.pop_front() {
                    self.index.remove(&old);
                }
            } else {
                break;
            }
        }
    }
}

// ───── Individual inbound stages ────────────────────────────────────

struct DecodeMw;
struct ExtractFieldsMw;
struct RecallGuardMw;
struct DedupMw;
struct SkipSelfMw;
struct ChatRoutingMw;
struct AccessGuardMw;
struct AutoSetHomeMw;
struct ExtractContentMw;
struct PlaceholderFilterMw;
struct OwnerCommandMw;
struct BuildSourceMw;
struct GroupAtGuardMw;
struct GroupAttributionMw;
struct ClassifyMsgTypeMw;
struct QuoteContextMw;
struct MediaResolveMw;

#[async_trait]
impl Middleware for DecodeMw {
    fn name(&self) -> &'static str {
        "decode"
    }
    async fn process(&self, _s: &PipelineState, _c: &mut PipelineCtx) -> MwResult {
        // Decoding happens before we build a PipelineCtx — this MW is a placeholder
        // so the stage list still has 17 entries (mirrors hermes-agent).
        MwResult::Continue
    }
}

#[async_trait]
impl Middleware for ExtractFieldsMw {
    fn name(&self) -> &'static str {
        "extract_fields"
    }
    async fn process(&self, _s: &PipelineState, _c: &mut PipelineCtx) -> MwResult {
        MwResult::Continue
    }
}

#[async_trait]
impl Middleware for RecallGuardMw {
    fn name(&self) -> &'static str {
        "recall_guard"
    }
    async fn process(&self, _s: &PipelineState, c: &mut PipelineCtx) -> MwResult {
        if c.msg.is_recall() {
            c.kind = MessageKind::Recall;
            return MwResult::Skip("recall_guard");
        }
        MwResult::Continue
    }
}

#[async_trait]
impl Middleware for DedupMw {
    fn name(&self) -> &'static str {
        "dedup"
    }
    async fn process(&self, s: &PipelineState, c: &mut PipelineCtx) -> MwResult {
        if c.msg.msg_id.is_empty() {
            return MwResult::Continue; // nothing to dedup on
        }
        let mut cache = s.dedup.write().await;
        if cache.check_and_insert(&c.msg.msg_id) {
            return MwResult::Skip("dedup");
        }
        MwResult::Continue
    }
}

#[async_trait]
impl Middleware for SkipSelfMw {
    fn name(&self) -> &'static str {
        "skip_self"
    }
    async fn process(&self, s: &PipelineState, c: &mut PipelineCtx) -> MwResult {
        if !s.bot_id.is_empty() && c.msg.from_account == s.bot_id {
            return MwResult::Skip("skip_self");
        }
        MwResult::Continue
    }
}

#[async_trait]
impl Middleware for ChatRoutingMw {
    fn name(&self) -> &'static str {
        "chat_routing"
    }
    async fn process(&self, _s: &PipelineState, c: &mut PipelineCtx) -> MwResult {
        c.source.is_group = c.msg.is_group();
        c.source.group_code = c.msg.group_code.clone();
        c.source.from_account = c.msg.from_account.clone();
        c.source.sender_nickname = c.msg.sender_nickname.clone();
        MwResult::Continue
    }
}

#[async_trait]
impl Middleware for AccessGuardMw {
    fn name(&self) -> &'static str {
        "access_guard"
    }
    async fn process(&self, s: &PipelineState, c: &mut PipelineCtx) -> MwResult {
        let (policy, allow_list, key) = if c.source.is_group {
            (s.group_access, &s.allowed_groups, &c.source.group_code)
        } else {
            (s.dm_access, &s.allowed_users, &c.source.from_account)
        };
        let pass = match policy {
            AccessPolicy::Open => true,
            AccessPolicy::Closed => false,
            AccessPolicy::Allowlist => allow_list.iter().any(|u| u == "*" || u == key),
        };
        if !pass {
            return MwResult::Skip("access_guard");
        }
        MwResult::Continue
    }
}

#[async_trait]
impl Middleware for AutoSetHomeMw {
    fn name(&self) -> &'static str {
        "auto_set_home"
    }
    async fn process(&self, s: &PipelineState, c: &mut PipelineCtx) -> MwResult {
        if !c.source.is_group {
            let mut home = s.home_chat.write().await;
            if home.is_none() {
                *home = Some(c.source.reply_target());
            }
        }
        MwResult::Continue
    }
}

#[async_trait]
impl Middleware for ExtractContentMw {
    fn name(&self) -> &'static str {
        "extract_content"
    }
    async fn process(&self, _s: &PipelineState, c: &mut PipelineCtx) -> MwResult {
        c.text = c.msg.extract_text();
        c.image_urls = c.msg.extract_image_urls();
        MwResult::Continue
    }
}

#[async_trait]
impl Middleware for PlaceholderFilterMw {
    fn name(&self) -> &'static str {
        "placeholder_filter"
    }
    async fn process(&self, _s: &PipelineState, c: &mut PipelineCtx) -> MwResult {
        let trimmed = c.text.trim();
        let is_placeholder = trimmed == "[image]" || trimmed == "[file]" || trimmed == "[图片]";
        if (trimmed.is_empty() || is_placeholder) && c.image_urls.is_empty() {
            return MwResult::Skip("placeholder_filter");
        }
        MwResult::Continue
    }
}

#[async_trait]
impl Middleware for OwnerCommandMw {
    fn name(&self) -> &'static str {
        "owner_command"
    }
    async fn process(&self, s: &PipelineState, c: &mut PipelineCtx) -> MwResult {
        if !s.owner_id.is_empty()
            && c.msg.from_account == s.owner_id
            && c.text.trim_start().starts_with('/')
        {
            c.is_owner_command = true;
        }
        MwResult::Continue
    }
}

#[async_trait]
impl Middleware for BuildSourceMw {
    fn name(&self) -> &'static str {
        "build_source"
    }
    async fn process(&self, _s: &PipelineState, _c: &mut PipelineCtx) -> MwResult {
        // Source already populated by ChatRoutingMw.
        MwResult::Continue
    }
}

#[async_trait]
impl Middleware for GroupAtGuardMw {
    fn name(&self) -> &'static str {
        "group_at_guard"
    }
    async fn process(&self, s: &PipelineState, c: &mut PipelineCtx) -> MwResult {
        if !c.source.is_group || !s.group_at_required {
            return MwResult::Continue;
        }
        let by_name = !s.bot_name.is_empty() && c.text.contains(&format!("@{}", s.bot_name));
        let by_id = !s.bot_id.is_empty() && c.text.contains(&format!("@{}", s.bot_id));
        let by_mention =
            !s.bot_id.is_empty() && c.text.contains(&format!("[at|userId:{}]", s.bot_id));
        c.is_at_bot = by_name || by_id || by_mention;
        if !c.is_at_bot && !c.is_owner_command {
            return MwResult::Skip("group_at_guard");
        }
        MwResult::Continue
    }
}

#[async_trait]
impl Middleware for GroupAttributionMw {
    fn name(&self) -> &'static str {
        "group_attribution"
    }
    async fn process(&self, s: &PipelineState, c: &mut PipelineCtx) -> MwResult {
        // Strip `@bot` from text and the TIM `[at|userId:…]` markup.
        if c.source.is_group && c.is_at_bot {
            if !s.bot_name.is_empty() {
                c.text = c.text.replace(&format!("@{}", s.bot_name), "");
            }
            if !s.bot_id.is_empty() {
                c.text = c.text.replace(&format!("@{}", s.bot_id), "");
                c.text = c.text.replace(&format!("[at|userId:{}]", s.bot_id), "");
            }
            c.text = c.text.trim().to_string();
        }
        MwResult::Continue
    }
}

#[async_trait]
impl Middleware for ClassifyMsgTypeMw {
    fn name(&self) -> &'static str {
        "classify_msg_type"
    }
    async fn process(&self, _s: &PipelineState, c: &mut PipelineCtx) -> MwResult {
        let has_text = !c.text.is_empty();
        let has_image = !c.image_urls.is_empty();
        let has_file = c.msg.msg_body.iter().any(|el| el.msg_type == tim::FILE);
        let has_sound = c.msg.msg_body.iter().any(|el| el.msg_type == tim::SOUND);
        c.kind = match (has_text, has_image, has_file, has_sound) {
            (_, true, _, _) if has_text => MessageKind::Mixed,
            (_, true, _, _) => MessageKind::Image,
            (_, _, true, _) => MessageKind::File,
            (_, _, _, true) => MessageKind::Voice,
            _ => MessageKind::Text,
        };
        MwResult::Continue
    }
}

#[async_trait]
impl Middleware for QuoteContextMw {
    fn name(&self) -> &'static str {
        "quote_context"
    }
    async fn process(&self, _s: &PipelineState, c: &mut PipelineCtx) -> MwResult {
        // The cloud_custom_data field carries a JSON quote envelope; for
        // now we just leave the raw payload accessible via `msg.cloud_custom_data`
        // for downstream tools. Full parsing is intentionally deferred —
        // hermes-agent does it lazily too.
        let _ = c;
        MwResult::Continue
    }
}

#[async_trait]
impl Middleware for MediaResolveMw {
    fn name(&self) -> &'static str {
        "media_resolve"
    }
    async fn process(&self, _s: &PipelineState, _c: &mut PipelineCtx) -> MwResult {
        // ybres:// resource URLs would be resolved here. Currently URLs
        // arrive pre-resolved from the server; expand later if needed.
        MwResult::Continue
    }
}

/// Composite pipeline = ordered Vec of inbound stages.
pub struct InboundPipeline {
    state: Arc<PipelineState>,
    stages: Vec<Box<dyn Middleware>>,
}

impl InboundPipeline {
    pub fn new(state: Arc<PipelineState>) -> Self {
        let stages: Vec<Box<dyn Middleware>> = vec![
            Box::new(DecodeMw),
            Box::new(ExtractFieldsMw),
            Box::new(RecallGuardMw),
            Box::new(DedupMw),
            Box::new(SkipSelfMw),
            Box::new(ChatRoutingMw),
            Box::new(AccessGuardMw),
            Box::new(AutoSetHomeMw),
            Box::new(ExtractContentMw),
            Box::new(PlaceholderFilterMw),
            Box::new(OwnerCommandMw),
            Box::new(BuildSourceMw),
            Box::new(GroupAtGuardMw),
            Box::new(GroupAttributionMw),
            Box::new(ClassifyMsgTypeMw),
            Box::new(QuoteContextMw),
            Box::new(MediaResolveMw),
        ];
        Self { state, stages }
    }

    /// Decode a biz push body, run it through every stage, return the outcome.
    ///
    /// The yuanbao gateway may push the biz body as either protobuf
    /// (`InboundMessagePush`) or a JSON string with the same field shape
    /// (snake_case + `log_ext.trace_id`). We sniff the first non-whitespace
    /// byte to pick the decoder — `{` means JSON, anything else is treated
    /// as protobuf. Mirrors plugin gateway.ts::wsPushToInboundMessage
    /// (l. 288), which tries protobuf first and falls back to JSON.
    pub async fn process(&self, biz_body: &[u8]) -> PipelineOutcome {
        let is_json = biz_body
            .iter()
            .find(|b| !b.is_ascii_whitespace())
            .map(|b| *b == b'{')
            .unwrap_or(false);

        let msg = if is_json {
            match decode_inbound_json(biz_body) {
                Ok(m) => m,
                Err(e) => return PipelineOutcome::Failed(e),
            }
        } else {
            match decode_inbound_push(biz_body) {
                Ok(m) => m,
                Err(e) => return PipelineOutcome::Failed(e),
            }
        };
        let mut ctx = PipelineCtx {
            msg,
            source: Source::default(),
            text: String::new(),
            image_urls: Vec::new(),
            is_at_bot: false,
            is_owner_command: false,
            kind: MessageKind::Text,
        };
        for stage in &self.stages {
            match stage.process(&self.state, &mut ctx).await {
                MwResult::Continue => {
                    trace!("[yuanbao:inbound] {} pass", stage.name());
                }
                MwResult::Skip(reason) => {
                    debug!("[yuanbao:inbound] {} filtered ({})", stage.name(), reason);
                    return PipelineOutcome::Filtered(reason);
                }
                MwResult::Abort(err) => return PipelineOutcome::Failed(err),
            }
        }
        PipelineOutcome::Dispatch(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(bot_id: &str) -> YuanbaoConfig {
        let mut c = YuanbaoConfig::default();
        c.app_key = "ak".into();
        c.ws_domain = "wss://x".into();
        c.token = "tok".into();
        c.bot_id = bot_id.into();
        c.bot_name = "bot".into();
        c.dm_access = "open".into();
        c.group_access = "open".into();
        c
    }

    fn ctx_with(msg: InboundMessage) -> PipelineCtx {
        PipelineCtx {
            msg,
            source: Source::default(),
            text: String::new(),
            image_urls: Vec::new(),
            is_at_bot: false,
            is_owner_command: false,
            kind: MessageKind::Text,
        }
    }

    #[tokio::test]
    async fn dedup_skips_repeat() {
        let state = PipelineState::new(&cfg("bot1"), "bot1".into());
        let mw = DedupMw;
        let msg = InboundMessage {
            msg_id: "m1".into(),
            ..Default::default()
        };
        let mut c1 = ctx_with(msg.clone());
        assert!(matches!(
            mw.process(&state, &mut c1).await,
            MwResult::Continue
        ));
        let mut c2 = ctx_with(msg);
        assert!(matches!(
            mw.process(&state, &mut c2).await,
            MwResult::Skip(_)
        ));
    }

    #[tokio::test]
    async fn access_guard_open() {
        let state = PipelineState::new(&cfg("bot1"), "bot1".into());
        let mw = AccessGuardMw;
        let mut c = ctx_with(InboundMessage {
            from_account: "alice".into(),
            ..Default::default()
        });
        c.source.is_group = false;
        c.source.from_account = "alice".into();
        assert!(matches!(
            mw.process(&state, &mut c).await,
            MwResult::Continue
        ));
    }

    #[tokio::test]
    async fn full_dm_dispatch() {
        let mut config = cfg("bot1");
        config.group_at_required = false;
        let state = PipelineState::new(&config, "bot1".into());
        let pipeline = InboundPipeline::new(state);
        let msg = InboundMessage {
            from_account: "alice".into(),
            to_account: "bot1".into(),
            msg_id: "hi".into(),
            msg_body: vec![MsgBodyElement {
                msg_type: "TIMTextElem".into(),
                msg_content: MsgContent {
                    text: Some("hello".into()),
                    ..Default::default()
                },
            }],
            ..Default::default()
        };
        let body = crate::openhuman::channels::providers::yuanbao::proto::encode_msg_body_element(
            &msg.msg_body[0],
        );
        // Synthesize an InboundMessagePush from scratch:
        use crate::openhuman::channels::providers::yuanbao::proto;
        let mut buf = Vec::new();
        let mut put_str = |fnum: u32, s: &str, b: &mut Vec<u8>| {
            proto::encode_varint(((fnum as u64) << 3) | 2, b);
            proto::encode_varint(s.len() as u64, b);
            b.extend_from_slice(s.as_bytes());
        };
        put_str(2, &msg.from_account, &mut buf);
        put_str(3, &msg.to_account, &mut buf);
        put_str(12, &msg.msg_id, &mut buf);
        proto::encode_varint(((13u64) << 3) | 2, &mut buf);
        proto::encode_varint(body.len() as u64, &mut buf);
        buf.extend_from_slice(&body);

        let outcome = pipeline.process(&buf).await;
        assert!(
            matches!(outcome, PipelineOutcome::Dispatch(_)),
            "got {:?}",
            outcome
        );
    }
}
