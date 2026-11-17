//! Local filesystem probes used by static diagnostics.

use std::io::Write;
use std::path::Path;

/// Creates, writes, and immediately removes a bounded temporary file in a
/// directory to verify the process can actually write there.
pub(crate) fn probe_directory_writable(directory: &Path) -> std::io::Result<()> {
    let mut file = tempfile::Builder::new()
        .prefix(".openhuman-doctor-")
        .tempfile_in(directory)?;
    file.as_file_mut().write_all(b"openhuman-write-probe")?;
    file.close()?;
    Ok(())
}
