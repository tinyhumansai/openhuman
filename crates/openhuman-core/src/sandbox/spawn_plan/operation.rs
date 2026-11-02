//! The concrete spawn a plan binds, in the canonical form the module hashes.
//!
//! A plan is valid for exactly one operation: changing the command, working
//! directory, environment or standard input yields a different canonical
//! document, so a plan made for one spawn can never admit another. Plan and consume
//! are both built from the same [`SpawnOperation`] by this one function.
//!
//! The command text rides in the document when it is short enough to read in the
//! audit trail; a longer one is bound by its digest and length, as are the
//! environment values (which may hold credentials) and standard input.

use std::path::Path;

use sha2::{Digest, Sha256};

/// Commands up to this many bytes travel verbatim; the module limits the whole
/// document to 64 KiB.
const INLINE_COMMAND_BYTES: usize = 16 * 1024;

/// One spawn as the host is about to perform it.
pub(crate) struct SpawnOperation<'a> {
    /// Shell text.
    pub(crate) command: &'a str,
    /// Working directory.
    pub(crate) cwd: &'a Path,
    /// Environment overrides, in any order.
    pub(crate) env: Vec<(String, String)>,
    /// Bytes written to standard input.
    pub(crate) stdin: Option<&'a [u8]>,
    /// Whether the shell keeps its own last-stage pipeline status.
    pub(crate) preserve_pipeline_status: bool,
}

fn hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

impl SpawnOperation<'_> {
    /// The canonical operation document.
    pub(crate) fn canonical(&self) -> Option<String> {
        let mut env: Vec<&(String, String)> = self.env.iter().collect();
        env.sort();
        let mut digest = Sha256::new();
        for (name, value) in &env {
            // Length-prefixed, so no name/value pair can be re-split into another.
            digest.update((name.len() as u64).to_be_bytes());
            digest.update(name.as_bytes());
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(value.as_bytes());
        }
        let inline = self.command.len() <= INLINE_COMMAND_BYTES;
        let mut document = serde_json::json!({
            "kind": "shell",
            "command": inline.then_some(self.command),
            "command_len": self.command.len(),
            "command_sha256": hex(self.command.as_bytes()),
            "cwd": self.cwd.to_str()?,
            "env_names": env.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(),
            "env_sha256": format!("{:x}", digest.finalize()),
            "stdin_len": self.stdin.map(<[u8]>::len),
            "stdin_sha256": self.stdin.map(hex),
            "preserve_pipeline_status": self.preserve_pipeline_status,
        });
        document.sort_all_objects();
        serde_json::to_string(&document).ok()
    }
}

#[cfg(test)]
#[path = "operation_tests.rs"]
mod tests;
