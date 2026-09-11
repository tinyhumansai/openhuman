use super::{resolve, unavailable, RuntimeCallError};
use crate::openhuman::config::Config;
use tinyruntime_bus::Language;

#[tokio::test]
async fn every_call_reports_the_missing_feature() {
    let error = resolve(&Config::default(), &Language::nodejs(), true)
        .await
        .expect_err("there is no module bus in this build");
    assert!(matches!(error, RuntimeCallError::Unavailable(_)));
    assert!(
        error.to_string().contains("modules feature"),
        "got `{error}`"
    );
}

#[test]
fn the_failure_renders_as_its_message_alone() {
    assert_eq!(unavailable().to_string(), super::MODULES_DISABLED_MESSAGE);
}
