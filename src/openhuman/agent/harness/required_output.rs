//! Required structured-output validation & repair (issue #4117).
//!
//! Some agent contracts mandate a JSON block — e.g. a `thoughts` block like
//! `{"thoughts": "…", "next_action": "…"}` — on **every** turn, because
//! downstream parsing/routing depends on it. Models frequently omit the block
//! entirely, leaving those consumers with nothing.
//!
//! This module supplies the pure primitives the turn engine uses to guarantee a
//! well-formed block on every accepted turn:
//!
//! * [`output_satisfies_contract`] — validate presence + shape of the block,
//! * [`repair_instruction`] — the corrective re-prompt that asks the model to
//!   re-emit its reply with the block, and
//! * [`synthesize_block`] — a minimal, schema-valid block used as a deterministic
//!   fallback when the re-prompt also omits it.
//!
//! The orchestration that ties these together (validate → re-prompt → synthesize)
//! lives on the session in `session::turn::session_io` so it can drive the extra
//! provider call and reconcile with streaming; keeping the logic here pure keeps
//! it unit-testable without a provider.

use crate::openhuman::config::RequiredOutputContract;

/// Whether `text` satisfies `contract`: it contains a JSON object carrying every
/// required key with a non-null value, in the expected leading position. An
/// inert contract (no non-blank keys) is treated as always satisfied so
/// enforcement is a no-op.
pub(crate) fn output_satisfies_contract(text: &str, contract: &RequiredOutputContract) -> bool {
    if !contract.is_active() {
        return true;
    }
    find_required_block(text, contract).is_some()
}

/// The required block when it appears in the expected **leading position** —
/// the *first* JSON value in `text` must be an object carrying every required
/// key with a non-null value — else `None`.
///
/// Issue #4117 mandates the block in a predictable position so downstream
/// parsing/routing can rely on it. Prose before the block is fine (prose is not
/// JSON, so the block is still the first extracted value), but a block buried
/// after *another* JSON object is rejected and gets repaired rather than
/// silently accepted. Reuses the harness JSON extractor so fenced, inline, and
/// whole-object replies are all recognised.
pub(crate) fn find_required_block(
    text: &str,
    contract: &RequiredOutputContract,
) -> Option<serde_json::Value> {
    let keys = contract.all_keys();
    if keys.is_empty() {
        return None;
    }
    let first = super::parse::extract_json_values(text).into_iter().next()?;
    let obj = first.as_object()?;
    let has_all = keys
        .iter()
        .all(|key| obj.get(key).is_some_and(|v| !v.is_null()));
    if has_all {
        Some(first)
    } else {
        None
    }
}

/// A minimal, schema-valid block synthesised when the model omits the block and
/// a corrective re-prompt fails to recover it. Every required key maps to an
/// empty string so downstream parsing always has a well-formed object to
/// consume. Returns `"{}"` only for an inert contract (which enforcement never
/// reaches).
pub(crate) fn synthesize_block(contract: &RequiredOutputContract) -> String {
    let mut obj = serde_json::Map::new();
    for key in contract.all_keys() {
        obj.insert(key, serde_json::Value::String(String::new()));
    }
    serde_json::to_string(&serde_json::Value::Object(obj)).unwrap_or_else(|_| "{}".to_string())
}

/// The corrective instruction that re-prompts the model to re-emit its reply
/// with the required block. Mirrors the iteration-cap checkpoint re-prompt: a
/// self-contained user turn appended after the omitting reply.
///
/// The model is asked to lead with the JSON object so the repaired reply
/// satisfies the leading-position rule directly, whether the caller keeps the
/// re-prompt as the whole reply (the non-streamed *replace* path) or appends it
/// after prose that was already streamed (the *append* path); see
/// `Agent::enforce_required_output`.
pub(crate) fn repair_instruction(contract: &RequiredOutputContract) -> String {
    let keys = contract.all_keys().join("\", \"");
    format!(
        "Your previous reply omitted the required JSON `{}` block that every turn must include. \
Reply again, leading with a single valid JSON object containing the keys \"{}\" — all present \
and non-null — then continue with your answer. Do not call any tools.",
        contract.block_key, keys
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thoughts_contract() -> RequiredOutputContract {
        RequiredOutputContract {
            block_key: "thoughts".into(),
            required_keys: vec!["next_action".into()],
        }
    }

    #[test]
    fn present_well_formed_block_satisfies_contract() {
        let contract = thoughts_contract();
        let text = "Sure! {\"thoughts\": \"planning\", \"next_action\": \"call tool\"}";
        assert!(output_satisfies_contract(text, &contract));
        assert!(find_required_block(text, &contract).is_some());
    }

    #[test]
    fn prose_only_reply_fails_validation() {
        let contract = thoughts_contract();
        assert!(!output_satisfies_contract(
            "Sure, I'll handle that.",
            &contract
        ));
    }

    #[test]
    fn block_missing_a_required_sibling_key_fails() {
        let contract = thoughts_contract();
        // Has `thoughts` but not `next_action`.
        let text = "{\"thoughts\": \"planning\"}";
        assert!(!output_satisfies_contract(text, &contract));
    }

    #[test]
    fn null_valued_required_key_fails() {
        let contract = RequiredOutputContract::new("thoughts");
        assert!(!output_satisfies_contract(
            "{\"thoughts\": null}",
            &contract
        ));
    }

    #[test]
    fn synthesized_block_satisfies_its_own_contract() {
        let contract = thoughts_contract();
        let synthesized = synthesize_block(&contract);
        assert!(
            output_satisfies_contract(&synthesized, &contract),
            "synthesized fallback must satisfy the contract it was built from: {synthesized}"
        );
    }

    #[test]
    fn leading_block_after_prose_is_accepted() {
        let contract = thoughts_contract();
        // Prose before the block is fine — prose is not JSON, so the block is
        // still the first extracted value.
        let text = "Here is my plan.\n{\"thoughts\": \"x\", \"next_action\": \"y\"}";
        assert!(output_satisfies_contract(text, &contract));
    }

    #[test]
    fn synthesized_block_prepended_to_prose_leads_correctly() {
        // The non-streamed *replace* fallback prepends a synthesized block to the
        // original prose; the block must be the first JSON value so the reply
        // validates.
        let contract = thoughts_contract();
        let repaired = format!("{}\n\n{}", synthesize_block(&contract), "Working on it.");
        assert!(output_satisfies_contract(&repaired, &contract));
    }

    #[test]
    fn block_buried_after_another_json_object_is_rejected() {
        let contract = thoughts_contract();
        // A different JSON object leads; the required block is second → rejected
        // so it gets repaired rather than silently accepted (issue #4117).
        let text = "{\"foo\": 1}\n{\"thoughts\": \"x\", \"next_action\": \"y\"}";
        assert!(!output_satisfies_contract(text, &contract));
    }

    #[test]
    fn blank_block_key_makes_contract_inert() {
        // A blank block key is inert even when sibling keys are listed — the
        // contract's defining key can never be enforced, so enforcement is
        // skipped instead of accepting a block missing that key.
        let contract = RequiredOutputContract {
            block_key: "   ".into(),
            required_keys: vec!["next_action".into()],
        };
        assert!(!contract.is_active());
        assert!(output_satisfies_contract(
            "{\"next_action\": \"y\"}",
            &contract
        ));
    }

    #[test]
    fn inert_contract_is_always_satisfied() {
        let contract = RequiredOutputContract::default();
        assert!(!contract.is_active());
        assert!(output_satisfies_contract("no block here", &contract));
        assert!(find_required_block("no block here", &contract).is_none());
    }

    #[test]
    fn all_keys_trims_and_dedupes() {
        let contract = RequiredOutputContract {
            block_key: "  thoughts  ".into(),
            required_keys: vec![
                "thoughts".into(),
                " next_action ".into(),
                "next_action".into(),
            ],
        };
        // block_key trimmed; duplicate `thoughts` and repeated `next_action`
        // collapse to a single occurrence each, order-preserving.
        assert_eq!(contract.all_keys(), vec!["thoughts", "next_action"]);
    }

    #[test]
    fn repair_instruction_names_every_required_key() {
        let contract = thoughts_contract();
        let instruction = repair_instruction(&contract);
        assert!(instruction.contains("thoughts"));
        assert!(instruction.contains("next_action"));
    }
}
