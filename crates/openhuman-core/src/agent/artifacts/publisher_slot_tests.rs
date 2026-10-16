use super::*;
use tiny_artifacts::{ArtifactRecord, ArtifactVersion, PublishOptions, PublishedLink};

struct FixedPublisher;

#[async_trait::async_trait]
impl ArtifactPublisher for FixedPublisher {
    async fn publish(
        &self,
        _record: &ArtifactRecord,
        _version: &ArtifactVersion,
        _options: &PublishOptions,
    ) -> anyhow::Result<PublishedLink> {
        anyhow::bail!("not used")
    }
}

#[test]
fn nothing_registered_offers_no_publisher() {
    assert!(artifact_publisher(&Config::default()).is_none());
}

#[test]
fn the_first_factory_that_answers_serves_sharing() {
    register_artifact_publisher("declines", |_| None);
    assert!(artifact_publisher(&Config::default()).is_none());
    register_artifact_publisher("answers", |_| Some(Arc::new(FixedPublisher)));
    assert!(artifact_publisher(&Config::default()).is_some());
    assert!(unregister_artifact_publisher("answers"));
    assert!(artifact_publisher(&Config::default()).is_none());
}
