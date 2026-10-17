use super::*;
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use tinychannels_bus::ChannelMessage;
use tokio::sync::mpsc;

struct MockChannel {
    sends: Arc<AtomicUsize>,
    last_target: Arc<Mutex<String>>,
}

#[async_trait]
impl Channel for MockChannel {
    fn name(&self) -> &str {
        "telegram"
    }
    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        *self.last_target.lock().unwrap() = format!(
            "{}|{}",
            message.recipient,
            message.idempotency_key.clone().unwrap_or_default()
        );
        Ok(())
    }
    async fn listen(&self, _tx: mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        Ok(())
    }
}

// One test: the bridge is process-global, so keep its steps sequential.
#[tokio::test]
async fn bridge_reads_appends_trims_and_sends() {
    let histories: ChannelHistories = Arc::new(Mutex::new(HashMap::new()));
    histories.lock().unwrap().insert(
        "k".into(),
        vec![ChatMessage::user("remind me"), ChatMessage::assistant("ok")],
    );
    let sends = Arc::new(AtomicUsize::new(0));
    let last_target = Arc::new(Mutex::new(String::new()));
    let ch: Arc<dyn Channel> = Arc::new(MockChannel {
        sends: Arc::clone(&sends),
        last_target: Arc::clone(&last_target),
    });
    let channels = Arc::new(HashMap::from([("telegram".to_string(), ch)]));
    register_channel_bridge(Arc::clone(&histories), channels, 3);

    assert_eq!(
        history_messages("k"),
        vec![
            ("user".to_string(), "remind me".to_string()),
            ("assistant".to_string(), "ok".to_string())
        ]
    );
    assert!(history_messages("missing").is_empty());

    assert!(append_assistant_message("k", "drink water"));
    assert!(append_assistant_message("k", "again"));
    let got = history_messages("k");
    assert_eq!(got.len(), 3, "trimmed to the registered maximum");
    assert_eq!(got.last().unwrap().1, "again");
    assert_eq!(got[0].1, "ok");

    send_to_channel("Telegram", "42", None, "hi", "cron:j:r1")
        .await
        .unwrap();
    assert_eq!(sends.load(Ordering::SeqCst), 1);
    assert_eq!(
        *last_target.lock().unwrap(),
        "42|cron:j:r1",
        "the send carries the run-specific idempotency key"
    );
    assert!(send_to_channel("slack", "42", None, "hi", "k")
        .await
        .is_err());
}
