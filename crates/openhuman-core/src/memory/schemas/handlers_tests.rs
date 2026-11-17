use super::engine_set_save_error;

#[test]
fn nonconflict_save_failures_remain_engine_errors() {
    let raw = engine_set_save_error(anyhow::anyhow!("disk unavailable"));
    let error = crate::core::StructuredRpcError::decode(&raw).unwrap();
    assert_eq!(error.data.as_ref().unwrap()["code"], "ENGINE");
    assert_eq!(
        error.message,
        "engine error: saving config failed: disk unavailable"
    );
}
