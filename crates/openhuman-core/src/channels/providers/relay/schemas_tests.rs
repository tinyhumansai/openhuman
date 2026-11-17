use super::*;
use crate::core::runtime::DispatchCtx;

#[test]
fn the_controller_is_channel_relay_inbound() {
    let controllers = all_relay_registered_controllers();
    assert_eq!(controllers.len(), 1);
    assert_eq!(
        controllers[0].rpc_method_name(),
        "openhuman.channel_relay_inbound"
    );
    let schema = &all_relay_controller_schemas()[0];
    let required: Vec<&str> = schema
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert_eq!(
        required,
        vec!["channel", "chat_id", "sender_id", "message_id", "text"]
    );
    assert_eq!(schemas("nope").function, "unknown");
}

#[test]
fn the_controller_is_registered() {
    assert!(crate::core::all::all_registered_controllers()
        .iter()
        .any(|c| c.rpc_method_name() == "openhuman.channel_relay_inbound"));
}

#[tokio::test]
async fn malformed_params_are_refused_before_any_work() {
    let mut params = Map::new();
    params.insert("channel".into(), Value::String("telegram".into()));
    let err = handle_relay_inbound(DispatchCtx::process_default(), params)
        .await
        .expect_err("missing fields");
    assert!(err.contains("invalid params"), "{err}");
}
