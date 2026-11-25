use super::*;

struct FakePort;

#[async_trait]
impl SocketPort for FakePort {
    async fn connect(&self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn connect_with_provider(&self, _: &str, _: TokenProvider) -> Result<(), String> {
        Ok(())
    }
    async fn disconnect(&self) -> Result<(), String> {
        Ok(())
    }
    async fn emit(&self, _: &str, _: serde_json::Value) -> Result<(), String> {
        Ok(())
    }
    async fn emit_with_ack(
        &self,
        _: &str,
        _: serde_json::Value,
        _: std::time::Duration,
    ) -> Result<serde_json::Value, String> {
        Ok(serde_json::Value::Null)
    }
    fn get_state(&self) -> SocketState {
        SocketState::default()
    }
    fn is_connected(&self) -> bool {
        false
    }
    fn is_live_for(&self, _: &str, _: &str) -> bool {
        false
    }
    fn status_for(&self, _: &str, _: &str) -> Option<ConnectionStatus> {
        None
    }
    fn is_loop_active(&self) -> bool {
        false
    }
    fn loop_stopped_on_failure(&self) -> bool {
        false
    }
    fn set_webhook_router(&self, _: Arc<crate::skills::webhooks::WebhookRouter>) {}
    fn webhook_router(&self) -> Option<Arc<crate::skills::webhooks::WebhookRouter>> {
        None
    }
}

#[tokio::test]
async fn socket_port_identity_guard_is_available_to_host_transactions() {
    let port = FakePort;
    let _guard = port.lock_identity_rebind().await;
    assert_eq!(port.get_state().status, ConnectionStatus::Disconnected);
}
