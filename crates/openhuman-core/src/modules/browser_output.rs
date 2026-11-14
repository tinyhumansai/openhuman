//! Screenshots held by TinyComputer: pulling one in chunks, checking it
//! against its handle, and releasing it however the read ends.

use tinycomputer_bus::browser::{names, OutputChunk, OutputId, OutputRef, OutputRequest};

use super::{BrowserCallError, BrowserClient};

/// The most bytes a screenshot may be, whatever its handle says: the module
/// caps an output well under this, so more is a broken reply.
pub(crate) const MAX_SCREENSHOT_BYTES: u64 = 32 * 1024 * 1024;

pub(super) fn output_error(message: impl Into<String>) -> BrowserCallError {
    BrowserCallError::Bus {
        name: "InvalidReply".into(),
        message: message.into(),
    }
}

/// Releases a held output when the screenshot call is dropped before it
/// could release it itself (a cancelled turn), so the module's slot is not
/// spent until it expires.
pub(super) struct ReleaseOnDrop {
    pub(super) client: Option<BrowserClient>,
    pub(super) output: OutputId,
    /// The caller's dispatch context, taken when the guard is made: the
    /// cleanup task needs the same tenant to reach the module.
    pub(super) ctx: crate::core::runtime::DispatchCtx,
}

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        let Some(client) = self.client.take() else {
            return;
        };
        let output = self.output.clone();
        if tokio::runtime::Handle::try_current().is_ok() {
            self.ctx.spawn(async move {
                let _released: Result<serde_json::Value, _> = client
                    .call(names::methods::RELEASE_OUTPUT, OutputRequest { output })
                    .await;
            });
        }
    }
}

pub(super) fn check_output_size(held: &OutputRef) -> Result<(), BrowserCallError> {
    if held.total_bytes > MAX_SCREENSHOT_BYTES {
        return Err(output_error(
            "the screenshot is larger than a screenshot can be",
        ));
    }
    Ok(())
}

/// Appends `chunk`, the next piece of `held`, to `bytes`.
pub(super) fn append_output_chunk(
    held: &OutputRef,
    bytes: &mut Vec<u8>,
    chunk: &OutputChunk,
) -> Result<(), BrowserCallError> {
    use base64::Engine as _;

    if chunk.id != held.id || chunk.offset != bytes.len() as u64 {
        return Err(output_error("a screenshot chunk is out of order"));
    }
    let data = base64::engine::general_purpose::STANDARD
        .decode(chunk.data.as_bytes())
        .map_err(|_| output_error("a screenshot chunk is not base64"))?;
    if data.is_empty() && !chunk.eof {
        return Err(output_error("a screenshot chunk is empty"));
    }
    if (bytes.len() + data.len()) as u64 > held.total_bytes {
        return Err(output_error(
            "the screenshot is longer than its handle says",
        ));
    }
    bytes.extend_from_slice(&data);
    Ok(())
}

/// Checks the whole of `bytes` against `held`'s size and SHA-256.
pub(super) fn finish_output(held: &OutputRef, bytes: &[u8]) -> Result<(), BrowserCallError> {
    use sha2::{Digest, Sha256};

    if bytes.len() as u64 != held.total_bytes {
        return Err(output_error(
            "the screenshot is shorter than its handle says",
        ));
    }
    if format!("{:x}", Sha256::digest(bytes)) != held.sha256.to_ascii_lowercase() {
        return Err(output_error("the screenshot does not match its SHA-256"));
    }
    Ok(())
}

/// The bytes `chunks` (read in order from offset zero) carry for `held`.
/// Once `complete`, they must be exactly the handle's size and SHA-256.
#[cfg(test)]
pub(crate) fn assemble_output(
    held: &OutputRef,
    chunks: &[OutputChunk],
    complete: bool,
) -> Result<Vec<u8>, BrowserCallError> {
    check_output_size(held)?;
    let mut bytes = Vec::new();
    for chunk in chunks {
        append_output_chunk(held, &mut bytes, chunk)?;
    }
    if complete {
        finish_output(held, &bytes)?;
    }
    Ok(bytes)
}

#[cfg(test)]
#[path = "browser_output_tests.rs"]
mod tests;
