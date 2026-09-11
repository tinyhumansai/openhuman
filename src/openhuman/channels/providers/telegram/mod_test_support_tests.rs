//! Debug-build seams for raw integration coverage of Telegram send helpers.
//! Delegates to the tinychannels transport crate where the logic now lives.

pub use tinychannels::providers::telegram::test_support::parse_reaction_marker_for_test;
