//! Host-side coverage for the n8n importer, for the one property the crate
//! cannot assert on its own.
//!
//! `tinyflows_catalog::import::n8n` tests its own mapping. What it cannot test
//! is that an imported graph then clears *this host's* gates: an import that
//! produced a graph `propose_workflow` / `revise_workflow` / `save_workflow`
//! would refuse is a broken import even though every mapping assertion passed.

#[cfg(test)]
#[path = "import_tests_tests.rs"]
mod tests;
