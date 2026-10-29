use super::*;
use std::cell::RefCell;

struct NoNetworkModel;

#[async_trait::async_trait]
impl ChatModel<()> for NoNetworkModel {
    async fn invoke(
        &self,
        _: &(),
        _: tinyinference_llm::model::ModelRequest,
    ) -> tinyinference_llm::Result<tinyinference_llm::model::ModelResponse> {
        Err(tinyinference_llm::Error::Model("unused test route".into()))
    }
}

#[test]
fn inspection_binds_one_credential_without_enabling_failover() {
    let built = RefCell::new(Vec::new());
    let result = build_cloud_model_with_keys(
        "inspection-model".into(),
        vec!["key-one".into(), "key-two".into()],
        false,
        |key| {
            built.borrow_mut().push(key.to_owned());
            Ok(Arc::new(NoNetworkModel) as Arc<dyn ChatModel<()>>)
        },
    );
    assert!(result.is_ok());
    assert_eq!(*built.borrow(), ["key-one"]);
}

#[test]
fn runtime_builds_one_native_model_per_distinct_credential() {
    let built = RefCell::new(Vec::new());
    let result = build_cloud_model_with_keys(
        "runtime-model".into(),
        vec!["key-one".into(), "key-two".into()],
        true,
        |key| {
            built.borrow_mut().push(key.to_owned());
            Ok(Arc::new(NoNetworkModel) as Arc<dyn ChatModel<()>>)
        },
    );
    assert!(result.is_ok());
    assert_eq!(*built.borrow(), ["key-one", "key-two"]);
}
