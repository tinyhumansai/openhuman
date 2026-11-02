use super::*;

use crate::modules::security::gap::{install_budget_port, BudgetOp, ScriptedBudgetPort};
use tinysecurity_bus::BudgetDecision2;

#[tokio::test]
async fn action_spending_refreshes_the_module_snapshot_for_sync_gates() {
    let port = ScriptedBudgetPort::replying(BudgetDecision2::Allow);
    let _installed = install_budget_port(port.clone());
    let policy = SecurityPolicy {
        enabled: true,
        max_actions_per_hour: 9,
        ..SecurityPolicy::default()
    };

    policy.spend_action("file_write").await.unwrap();

    assert!(policy.record_action());
    assert!(policy.record_action());
    assert_eq!(
        port.asked(),
        vec![(BudgetOp::Spend, 9)],
        "action permission and accounting must come from the bus"
    );
}

#[tokio::test]
async fn a_spent_module_budget_is_reported_with_the_module_limit() {
    let port = ScriptedBudgetPort::replying(BudgetDecision2::RateLimited { max_per_hour: 4 });
    let _installed = install_budget_port(port);
    let policy = SecurityPolicy {
        enabled: true,
        max_actions_per_hour: 9,
        ..SecurityPolicy::default()
    };

    let error = policy.spend_action("file_write").await.unwrap_err();

    assert!(error.contains("4 actions/hour"), "{error}");
    assert!(error.contains("Rate limit exceeded"), "{error}");
    assert!(
        policy.is_rate_limited(),
        "sync consumers read the cached peek/spend answer"
    );
}

#[tokio::test]
async fn checking_the_budget_uses_peek_without_spending_an_action() {
    let port = ScriptedBudgetPort::replying(BudgetDecision2::Allow);
    let _installed = install_budget_port(port.clone());
    let policy = SecurityPolicy {
        enabled: true,
        max_actions_per_hour: 9,
        ..SecurityPolicy::default()
    };

    assert_eq!(
        policy.peek_action_budget().await.unwrap(),
        BudgetDecision2::Allow
    );
    assert!(!policy.is_rate_limited());
    assert_eq!(port.asked(), vec![(BudgetOp::Peek, 9)]);
}
