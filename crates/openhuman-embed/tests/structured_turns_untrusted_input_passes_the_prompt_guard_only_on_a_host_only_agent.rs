mod common;
#[path = "support/structured_turns.rs"]
mod scenarios;

#[test]
fn untrusted_input_passes_the_prompt_guard_only_on_a_host_only_agent() {
    scenarios::untrusted_input_passes_the_prompt_guard_only_on_a_host_only_agent();
}
