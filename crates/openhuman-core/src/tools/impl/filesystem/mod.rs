//! Host adapter for the filesystem tools.
//!
//! The tools themselves (`file_read`, `file_write`, `edit_file`,
//! `apply_patch`, `grep`, `glob`, `list_files`, `csv_export`, `read_diff`,
//! `git_operations`, `run_linter`, `run_tests`, `update_memory_md`) live in
//! `tinytools_std::filesystem` and are registered by `tools/ops.rs`. What stays
//! here is the policy they consult: [`gate`] implements
//! `tinytools_std::filesystem::FsGate` for [`crate::security::SecurityPolicy`],
//! so the autonomy flag, the the protected-path floor floor, `workspace_only`,
//! trusted roots and the approval gate remain host decisions. The tools only
//! ask.

mod gate;

#[cfg(test)]
#[path = "gate_tests.rs"]
mod gate_tests;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
