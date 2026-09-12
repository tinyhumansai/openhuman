//! The filesystem write the tools go through.
//!
//! Every tool that reports "the write failed" has a branch that only runs when
//! the OS refuses, and that branch is the one worth asserting: it decides
//! whether a failed edit is reported honestly or fabricated as a success.
//!
//! Provoking a real refusal from a test is where this gets awkward. A
//! mode-bit denial is a discretionary check, and a root test runner carries
//! `CAP_DAC_OVERRIDE` and writes through it — so a fixture built on
//! permissions succeeds under CI, and the case either fails on a premise that
//! never held or is skipped and asserts nothing at all. Both read as coverage.
//!
//! So the write is a seam instead. Production installs the OS; a test installs
//! one that refuses, and the reporting branch becomes ordinary to assert on
//! any platform and as any user.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;

/// Where a tool's file writes land.
#[async_trait]
pub trait FileSink: Send + Sync {
    async fn write(&self, path: &Path, contents: &[u8]) -> std::io::Result<()>;
}

/// The real filesystem.
pub struct OsFileSink;

#[async_trait]
impl FileSink for OsFileSink {
    async fn write(&self, path: &Path, contents: &[u8]) -> std::io::Result<()> {
        tokio::fs::write(path, contents).await
    }
}

/// The sink a tool uses unless a caller installs another one.
pub fn os_sink() -> Arc<dyn FileSink> {
    Arc::new(OsFileSink)
}

/// A sink that refuses every write with `kind`, for asserting the branch a
/// tool takes when the filesystem says no.
#[cfg(test)]
pub struct RefusingSink(pub std::io::ErrorKind);

#[cfg(test)]
#[async_trait]
impl FileSink for RefusingSink {
    async fn write(&self, path: &Path, _contents: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::new(
            self.0,
            format!("refused by the test sink: {}", path.display()),
        ))
    }
}
