//! The host's `Channel` seam is the contract crate's trait, not a copy.
use super::*;
use std::sync::Mutex;

#[derive(Default)]
struct Recorder {
    sent: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl tinychannels_bus::Channel for Recorder {
    fn name(&self) -> &str {
        "recorder"
    }
    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        self.sent.lock().unwrap().push(message.content.clone());
        Ok(())
    }
    async fn listen(
        &self,
        _tx: tokio::sync::mpsc::Sender<tinychannels_bus::ChannelMessage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn host_send_ext_routes_through_the_contract_trait() {
    let ch = Recorder::default();
    ChannelSendExt::send_with_outbound_intent(&ch, &SendMessage::new("hi", "u1"))
        .await
        .unwrap();
    assert_eq!(ch.sent.lock().unwrap().as_slice(), ["hi"]);
}

#[test]
fn channel_vocabulary_is_the_contract_crates_own_types() {
    use std::any::TypeId;
    assert_eq!(
        TypeId::of::<dyn Channel>(),
        TypeId::of::<dyn tinychannels_bus::Channel>()
    );
    assert_eq!(
        TypeId::of::<SendMessage>(),
        TypeId::of::<tinychannels_bus::SendMessage>()
    );
}
