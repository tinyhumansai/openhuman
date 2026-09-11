use super::*;

fn thoughts_contract() -> RequiredOutput {
    RequiredOutput {
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
    let contract = RequiredOutput::new("thoughts");
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
    let contract = RequiredOutput {
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
    let contract = RequiredOutput::default();
    assert!(!contract.is_active());
    assert!(output_satisfies_contract("no block here", &contract));
    assert!(find_required_block("no block here", &contract).is_none());
}

#[test]
fn all_keys_trims_and_dedupes() {
    let contract = RequiredOutput {
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
