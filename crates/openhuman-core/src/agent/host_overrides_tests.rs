use super::*;
use async_trait::async_trait;

struct Hook;
#[async_trait]
impl tinyagents_harness::host::LearningSink for Hook {
    async fn on_turn_complete(
        &self,
        _: &tinyagents_harness::host::TurnSummary,
    ) -> Result<(), tinyagents_session::TinyAgentsError> {
        Ok(())
    }
}

#[test]
fn local_hooks_do_not_escape_their_agent_and_can_be_removed() {
    let a = HostOverrides::default();
    let b = HostOverrides::default();
    a.post_turn_hook("local", Some(Arc::new(Hook)));
    assert_eq!(a.post_turn_hooks().len(), 1);
    assert!(b.post_turn_hooks().is_empty());
    a.post_turn_hook("local", None);
    assert!(a.post_turn_hooks().is_empty());
}
