//! Host-side coverage for the copilot's tool-facing wording.
//!
//! `tinyflows-copilot` pins what the standing archetype says. It cannot pin
//! what *this host's* tool descriptions say, and the two have to agree: the
//! archetype tells the model that an `agent_ref` step runs the selected
//! specialist's full tool loop, and `list_agent_profiles`' own description used
//! to contradict that with stale "follow-up" / "for now" wording.

#[cfg(test)]
#[path = "tool_wording_tests_tests.rs"]
mod tests;
