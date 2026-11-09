//! Inline skill guides: the playbooks that replaced single-belt specialists.

use super::*;

/// ~550 tokens at the o200k average of ~4 bytes per token. A guide replaced a
/// specialist's whole system prompt, and it is paid once per load instead of
/// once per turn — but only while it stays a playbook, not a manual.
const GUIDE_BUDGET_BYTES: usize = 2_400;

/// The skills that replaced a removed specialist, each with a playbook.
const GUIDED_PACKS: &[&str] = &["coding", "web3", "system", "scheduling", "docs", "mcp"];

#[test]
fn the_skills_that_replaced_specialists_carry_a_guide() {
    for id in GUIDED_PACKS {
        let pack = pack(id).unwrap_or_else(|| panic!("missing pack `{id}`"));
        assert!(
            !pack.guide.trim().is_empty(),
            "skill `{id}` replaced a specialist and must carry its playbook"
        );
    }
}

#[test]
fn every_guide_stays_within_its_token_budget() {
    for pack in registry::PACKS {
        assert!(
            pack.guide.len() <= GUIDE_BUDGET_BYTES,
            "skill `{}` guide is {} bytes; budget is {GUIDE_BUDGET_BYTES}",
            pack.id,
            pack.guide.len()
        );
    }
}

#[test]
fn no_guide_names_a_removed_specialists_hand_off() {
    // Each of these was the delegate of an agent the skills replaced. A guide
    // that still names one teaches the model a tool that does not exist.
    const REMOVED: &[&str] = &[
        "`do_crypto`",
        "`manage_settings`",
        "`schedule_task`",
        "`run_code`",
        "`review_code`",
        "`use_mcp_server`",
        "`ask_docs`",
        "`run_skill`",
        "`plan`",
        "ask_user_clarification",
    ];
    for pack in registry::PACKS {
        for stale in REMOVED {
            assert!(
                !pack.guide.contains(stale),
                "skill `{}` guide names {stale}",
                pack.id
            );
        }
    }
}

#[test]
fn every_guided_pack_tool_is_named_in_its_guide_or_by_family() {
    // The guide is the only prose the model reads before calling a pack tool
    // it could not otherwise see; a member it never mentions is one the model
    // has no reason to reach for. A family wildcard (`config_get_*`) counts.
    for id in GUIDED_PACKS {
        let pack = pack(id).unwrap();
        for tool in pack.tools {
            let named = pack.guide.contains(&format!("`{tool}`"))
                || pack.guide.split('`').any(|token| {
                    token
                        .strip_suffix('*')
                        .is_some_and(|prefix| !prefix.is_empty() && tool.starts_with(prefix))
                });
            let optional = matches!(*tool, "mcp_registry_disconnect" | "mcp_registry_uninstall");
            assert!(
                named || optional,
                "skill `{id}` guide never names its tool `{tool}`"
            );
        }
    }
}

#[tokio::test]
async fn loading_a_skill_prints_its_guide_before_the_tool_schemas() {
    let web3 = pack("web3").unwrap();
    let (tools, _handle) = registry_with_all(&["wallet_status", "web3_swap_quote"]);
    let result = find(&tools, USE_SKILL)
        .execute(json!({"skill": "web3"}))
        .await
        .unwrap();
    assert!(!result.is_error);
    let text = format!("{:?}", result.content);
    let opening: String = web3.guide.trim().chars().take(40).collect();
    let guide_at = text.find(&opening).expect("guide rendered");
    let schema_at = text.find("## `wallet_status`").expect("tool rendered");
    assert!(
        guide_at < schema_at,
        "guide must precede the schemas:\n{text}"
    );
}
