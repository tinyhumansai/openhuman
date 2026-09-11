// `Agent`'s tinyagents-backed chat turn, split per the repo's
// `_part_NN.rs` convention because the combined body exceeds the 750-line
// ceiling enforced by `scripts/ci/check-openhuman-rust-layout.mjs`.
//
// Part 01 is the turn driver (`run_turn_via_tinyagents_session`); part 02 is
// the agent-experience / triggered-memory context injection it calls. Both
// are `impl Agent` blocks spliced into `core.rs`, so this file is text, not
// a module — the fragments' own line numbers are the ones to cite.
include!("core_session_part_01.rs");
include!("core_session_part_02.rs");
