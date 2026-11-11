//! The dispatch loop's per-conversation inbound debounce: bursts merge into
//! one turn, the media window stretches, windows flush at their deadline or on
//! shutdown, disabled config keeps one turn per message in order, and the
//! intake budget back-pressures the inbound queue.
//!
//! Turns are observed through the stubbed `agent.run_turn` bus handler; the
//! messages arrive already prepared, so no module call runs under paused time.

use super::super::context::{ChannelRuntimeContext, CHANNEL_MESSAGE_TIMEOUT_SECS};
use super::super::runtime::{run_message_dispatch_loop, ChannelHandle, RuntimeChannelMessage};
use super::common::RecordingChannel;
use crate::agent::bus::{mock_agent_run_turn, AgentTurnRequest, AgentTurnResponse};
use crate::config::InboundDebounceConfig;
use crate::core::bus_testing::MockBusGuard;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinHandle;

#[derive(Default)]
struct Turns {
    users: Mutex<Vec<String>>,
    active: AtomicUsize,
    peak: AtomicUsize,
}

impl Turns {
    fn count(&self) -> usize {
        self.users.lock().unwrap().len()
    }

    fn user(&self, turn: usize) -> String {
        self.users.lock().unwrap()[turn].clone()
    }
}

/// How a stubbed turn waits before it answers.
#[derive(Clone)]
struct Gate {
    permits: Arc<Semaphore>,
    /// Park every turn, not just the first.
    all_turns: bool,
}

impl Gate {
    fn first_turn() -> Self {
        Self {
            permits: Arc::new(Semaphore::new(0)),
            all_turns: false,
        }
    }

    fn every_turn() -> Self {
        Self {
            permits: Arc::new(Semaphore::new(0)),
            all_turns: true,
        }
    }

    fn open(&self) {
        self.permits.add_permits(10_000);
    }
}

/// Stub the turn handler. A `gate` parks the first (or every) turn until it
/// is opened.
async fn install_handler(turns: Arc<Turns>, gate: Option<Gate>) -> MockBusGuard {
    mock_agent_run_turn(move |req: AgentTurnRequest| {
        let turns = Arc::clone(&turns);
        let gate = gate.clone();
        async move {
            let user = req
                .history
                .iter()
                .rev()
                .find(|message| message.role == "user")
                .map(|message| message.content.clone())
                .unwrap_or_default();
            let index = {
                let mut users = turns.users.lock().unwrap();
                users.push(user);
                users.len()
            };
            let active = turns.active.fetch_add(1, Ordering::SeqCst) + 1;
            turns.peak.fetch_max(active, Ordering::SeqCst);
            if let Some(gate) = gate.filter(|gate| gate.all_turns || index == 1) {
                gate.permits
                    .acquire()
                    .await
                    .expect("gate stays open")
                    .forget();
            }
            turns.active.fetch_sub(1, Ordering::SeqCst);
            Ok(AgentTurnResponse::new(format!("reply {index}")))
        }
    })
    .await
}

fn runtime() -> (Arc<RecordingChannel>, Arc<ChannelRuntimeContext>) {
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel = Arc::new(ChannelHandle::Test(channel_impl.clone()));
    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);
    let ctx = Arc::new(ChannelRuntimeContext {
        relay_thread: None,
        channels_by_name: Arc::new(channels_by_name),
        module_objects: Arc::new(HashMap::new()),
        turn_model_source: Some(crate::agent::tinyagents::TurnModelSource::from_model(
            Arc::new(super::common::SlowModel {
                delay: Duration::from_millis(1),
            }),
        )),
        default_provider: Arc::new("test-provider".to_string()),
        tools_registry: Arc::new(vec![]),
        system_prompt: crate::channels::ChannelSystemPrompt::fixed("test-system-prompt"),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        max_tool_iterations: 10,
        turn_model_source_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_url: None,
        inference_url: None,
        reliability: Arc::new(crate::config::ReliabilityConfig::default()),
        workspace_dir: Arc::new(std::env::temp_dir()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        multimodal: crate::config::MultimodalConfig::default(),
        multimodal_files: crate::config::MultimodalFileConfig::default(),
        config: None,
        core: None,
    });
    (channel_impl, ctx)
}

struct Running {
    tx: mpsc::Sender<RuntimeChannelMessage>,
    task: JoinHandle<()>,
    next_ts: u64,
}

impl Running {
    fn start(ctx: Arc<ChannelRuntimeContext>, config: InboundDebounceConfig) -> Self {
        let (tx, rx) = mpsc::channel(32);
        let task = crate::core::runtime::DispatchCtx::ambient()
            .spawn(run_message_dispatch_loop(rx, ctx, 4, config));
        Self {
            tx,
            task,
            next_ts: 1,
        }
    }

    fn prepared(&mut self, sender: &str, content: &str) -> RuntimeChannelMessage {
        let ts = self.next_ts;
        self.next_ts += 1;
        let message = tinychannels_bus::ChannelMessage {
            reply_to_id: None,
            topic_id: None,
            id: format!("m{ts}"),
            sender: sender.to_string(),
            reply_target: sender.to_string(),
            content: content.to_string(),
            channel: "test-channel".to_string(),
            timestamp: ts,
            thread_ts: None,
            sender_name: None,
        };
        let envelope = tinychannels::inbound_envelope_from_legacy_message(&message);
        let history_key = format!("test-channel_{sender}_{sender}");
        let prepared = tinychannels_bus::module_inbound::PreparedChannelInbound {
            message: message.clone(),
            envelope: envelope.clone(),
            history_key: history_key.clone(),
            history_key_ignores_thread: false,
            thread_id: history_key.clone(),
            client_id: "test-channel".to_string(),
            session_key: history_key,
        };
        RuntimeChannelMessage {
            message,
            inbound_envelope: Some(envelope),
            authenticated_relay: false,
            prepared_inbound: Some(prepared),
            imported_history: Vec::new(),
            legacy_history: None,
            progressive_target: None,
        }
    }

    async fn send(&mut self, sender: &str, content: &str) {
        let message = self.prepared(sender, content);
        self.tx.send(message).await.unwrap();
    }

    async fn finish(self) {
        drop(self.tx);
        tokio::time::timeout(Duration::from_secs(600), self.task)
            .await
            .expect("dispatch loop drains")
            .expect("dispatch loop does not panic");
    }
}

async fn sleep_ms(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[tokio::test(start_paused = true)]
async fn two_quick_messages_from_one_sender_become_one_turn() {
    let turns = Arc::new(Turns::default());
    let _guard = install_handler(Arc::clone(&turns), None).await;
    let (channel, ctx) = runtime();
    let mut run = Running::start(ctx, InboundDebounceConfig::default());

    run.send("alice", "first").await;
    sleep_ms(500).await;
    run.send("alice", "second").await;
    sleep_ms(1000).await;
    assert_eq!(turns.count(), 0, "still inside the quiet window");
    sleep_ms(2000).await;

    assert_eq!(turns.count(), 1);
    assert!(
        turns.user(0).ends_with("first\nsecond"),
        "{}",
        turns.user(0)
    );
    run.finish().await;
    assert_eq!(turns.count(), 1);
    assert_eq!(channel.sent_messages.lock().await.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn text_after_an_image_joins_it_through_the_media_window() {
    let turns = Arc::new(Turns::default());
    let _guard = install_handler(Arc::clone(&turns), None).await;
    let (_channel, ctx) = runtime();
    let mut run = Running::start(ctx, InboundDebounceConfig::default());

    run.send("alice", "[IMAGE:https://example.com/a.png]\nlook")
        .await;
    sleep_ms(3000).await;
    assert_eq!(
        turns.count(),
        0,
        "the media window outlasts the text window"
    );
    run.send("alice", "what is this?").await;
    sleep_ms(5000).await;

    assert_eq!(turns.count(), 1);
    let user = turns.user(0);
    assert!(user.contains("look\nwhat is this?"), "{user}");
    run.finish().await;
}

#[tokio::test(start_paused = true)]
async fn messages_during_a_running_turn_are_not_held_behind_it() {
    let turns = Arc::new(Turns::default());
    let gate = Gate::first_turn();
    let _guard = install_handler(Arc::clone(&turns), Some(gate.clone())).await;
    let (_channel, ctx) = runtime();
    let mut run = Running::start(ctx, InboundDebounceConfig::default());

    run.send("alice", "first").await;
    sleep_ms(2000).await;
    assert_eq!(turns.count(), 1, "first turn is running");
    run.send("alice", "second").await;
    sleep_ms(200).await;
    run.send("alice", "third").await;
    sleep_ms(3000).await;

    // The turn runner owns queueing and steering per conversation: the
    // debouncer only merges intake, so the next burst starts its own turn once
    // its quiet window closes, without waiting for the running turn.
    assert_eq!(turns.count(), 2);
    assert!(
        turns.user(1).ends_with("second\nthird"),
        "{}",
        turns.user(1)
    );

    gate.open();
    run.finish().await;
    assert_eq!(turns.count(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_steady_stream_flushes_at_max_wait() {
    let turns = Arc::new(Turns::default());
    let _guard = install_handler(Arc::clone(&turns), None).await;
    let (_channel, ctx) = runtime();
    let mut run = Running::start(ctx, InboundDebounceConfig::default());

    for index in 0..12 {
        run.send("alice", &format!("part {index}")).await;
        sleep_ms(900).await;
    }

    assert_eq!(
        turns.count(),
        1,
        "deadline at 10s, quiet window would be 11.4s"
    );
    let user = turns.user(0);
    assert!(
        user.contains("part 0\n") && user.ends_with("part 11"),
        "{user}"
    );
    run.finish().await;
}

#[tokio::test(start_paused = true)]
async fn disabled_config_runs_one_turn_per_message_in_order() {
    let turns = Arc::new(Turns::default());
    let _guard = install_handler(Arc::clone(&turns), None).await;
    let (_channel, ctx) = runtime();
    let mut run = Running::start(ctx, InboundDebounceConfig::disabled());

    run.send("alice", "one").await;
    run.send("alice", "two").await;
    sleep_ms(100).await;
    run.finish().await;

    // Disabled merging means one turn per message. The loop hands them to the
    // turn runner in receive order (admission order is covered by the
    // `in_order` tests); the stub bus handler is reached over real async IO, so
    // the order it observes them in is not asserted here.
    assert_eq!(turns.count(), 2);
    let mut users = [turns.user(0), turns.user(1)];
    users.sort();
    assert!(users[0].ends_with("one"), "{users:?}");
    assert!(users[1].ends_with("two"), "{users:?}");
}

#[tokio::test(start_paused = true)]
async fn different_senders_are_never_merged() {
    let turns = Arc::new(Turns::default());
    let _guard = install_handler(Arc::clone(&turns), None).await;
    let (channel, ctx) = runtime();
    let mut run = Running::start(ctx, InboundDebounceConfig::default());

    run.send("alice", "from alice").await;
    run.send("bob", "from bob").await;
    sleep_ms(2000).await;

    assert_eq!(turns.count(), 2);
    let mut users = [turns.user(0), turns.user(1)];
    users.sort();
    assert!(users[0].ends_with("from alice"));
    assert!(users[1].ends_with("from bob"));
    run.finish().await;
    assert_eq!(channel.sent_messages.lock().await.len(), 2);
}

#[tokio::test(start_paused = true)]
async fn closing_the_queue_flushes_buffered_messages() {
    let turns = Arc::new(Turns::default());
    let _guard = install_handler(Arc::clone(&turns), None).await;
    let (_channel, ctx) = runtime();
    let mut run = Running::start(ctx, InboundDebounceConfig::default());

    run.send("alice", "a").await;
    run.send("alice", "b").await;
    run.finish().await;

    assert_eq!(turns.count(), 1);
    assert!(turns.user(0).ends_with("a\nb"), "{}", turns.user(0));
}

#[tokio::test(start_paused = true)]
async fn a_full_intake_budget_stops_receiving_until_turns_drain() {
    let turns = Arc::new(Turns::default());
    let gate = Gate::every_turn();
    let _guard = install_handler(Arc::clone(&turns), Some(gate.clone())).await;
    let (_channel, ctx) = runtime();
    let config = InboundDebounceConfig {
        max_batch_messages: 1,
        ..InboundDebounceConfig::default()
    };
    let mut run = Running::start(ctx, config);

    run.send("alice", "first").await;
    sleep_ms(2000).await;
    assert_eq!(turns.count(), 1);

    let mut accepted = 0;
    for i in 0..200 {
        let message = run.prepared("alice", &format!("queued {i}"));
        if run.tx.try_send(message).is_err() {
            break;
        }
        accepted += 1;
        sleep_ms(1).await;
    }
    assert!(
        accepted < 200,
        "the loop stops receiving once its intake budget is full"
    );

    gate.open();
    run.finish().await;
    assert_eq!(turns.count(), 1 + accepted);
}
