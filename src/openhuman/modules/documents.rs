//! Calling the `tinydocs` module: the three document operations, over the bus.
//!
//! Each function here is the host half of one method on
//! `ai.tinyhumans.tinydocs.Documents`. They exist so the three tools that need a
//! document do not each have to know about streams, held outputs, or wire error
//! names — a tool asks for bytes and gets bytes or a reason.
//!
//! # The shape of a call
//!
//! Inbound payloads ride a `TinyBus` stream opened alongside the call, so a
//! `.pdf` or a set of slide images is never squeezed through a 16 MiB JSON
//! frame. Outbound payloads cannot use a stream — a served object has no way to
//! open one back to its caller — so a produced document is held by the module
//! and pulled here with `ReadOutput`, then released.
//!
//! The release is in a `defer`-shaped position on purpose: the module bounds what
//! it holds and expires it, but leaving a document to time out costs its budget
//! in the meantime, and the next caller sees a full store rather than a slot.
//!
//! # Deadlines belong to the caller
//!
//! Nothing here imposes one. The document tools already wrap their calls in a
//! `tokio::time::timeout` chosen for what the tool is doing, and a second
//! deadline underneath would make the effective limit the smaller of two numbers
//! nobody picked together.

use crate::openhuman::tools::implementations::document::format::spec::{
    DocumentSpec, WirePresentationSpec,
};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde::Deserialize;
use tinybus::stream::StreamRef;
use tinydocs_bus::names::methods;

use super::{host, ops, registry};
use crate::openhuman::config::Config;

/// Registry id of the module these calls go to.
const MODULE_ID: &str = "tinydocs";

/// How much of a held document to pull per `ReadOutput`.
///
/// Below the module's own per-chunk cap with room for the base64 expansion and
/// the JSON envelope around it.
const READ_CHUNK: u64 = 1024 * 1024;

/// Largest document this host will assemble from a module.
///
/// Matches the module's own per-output cap, and is enforced here anyway. A
/// declared length is a number a module sent us: trusting it for a
/// `Vec::with_capacity` turns a wrong or hostile value into an allocation
/// failure, which aborts the process rather than returning an error. The rest of
/// `read_all` already treats the declared length as possibly wrong; this applies
/// the same distrust to the allocation.
const MAX_DOCUMENT_BYTES: u64 = 64 * 1024 * 1024;

/// Why a document call did not produce bytes.
///
/// Three variants rather than one string because the three tools that call this
/// map them onto three different agent-facing shapes: a spec the model can fix,
/// a failure it cannot, and a capability that is not present at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentCallError {
    /// The module is not loaded and cannot be: unsupported host, downloads off,
    /// disabled in config, or a load that already failed in this process.
    Unavailable(String),
    /// The spec was rejected. A model can act on this.
    InvalidInput(String),
    /// Synthesis, extraction, or the transfer itself failed.
    Failed(String),
}

impl std::fmt::Display for DocumentCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) | Self::InvalidInput(message) | Self::Failed(message) => {
                f.write_str(message)
            }
        }
    }
}

/// A handle to a document the module is holding for us.
#[derive(Debug, Deserialize)]
struct OutputRef {
    output_id: String,
    total_bytes: u64,
    sha256: String,
}

/// Generate a `.docx` from `spec`.
///
/// # Errors
///
/// [`DocumentCallError`] describing whether the spec, the module, or the
/// synthesis was at fault.
pub async fn generate_docx(
    config: &Config,
    spec: &DocumentSpec,
) -> Result<Vec<u8>, DocumentCallError> {
    let (runtime, record) = ready(config).await?;
    let proxy = proxy(runtime, record)?;
    let handle: OutputRef = proxy
        .call(methods::GENERATE_DOCX, (spec,))
        .await
        .map_err(|error| classify(&error))?;
    collect(&proxy, handle).await
}

/// Generate a `.pptx` from `deck`, streaming `images` alongside the call.
///
/// `images` is every slide image concatenated in slide order; the `byte_len` on
/// each entry in `deck` says where one ends and the next begins.
///
/// # Errors
///
/// [`DocumentCallError`], including an `InvalidInput` if `images` does not add
/// up to the lengths `deck` declares.
pub async fn generate_pptx(
    config: &Config,
    deck: &WirePresentationSpec,
    images: &[u8],
) -> Result<Vec<u8>, DocumentCallError> {
    let (runtime, record) = ready(config).await?;
    let proxy = proxy(runtime, record)?;

    let handle: OutputRef = if images.is_empty() {
        // A text-only deck opens no stream: there is nothing to send, and an
        // empty stream is a round trip for nothing.
        proxy
            .call(methods::GENERATE_PPTX, (deck, Option::<StreamRef>::None))
            .await
            .map_err(|error| classify(&error))?
    } else {
        let (destination, path, interface) = address(record)?;
        runtime
            .connection()
            .call_with_stream(
                destination,
                path,
                interface,
                member(methods::GENERATE_PPTX)?,
                |stream| serde_json::json!([deck, stream]),
                images,
            )
            .await
            .map_err(|error| classify(&error))?
    };
    collect(&proxy, handle).await
}

/// Extract the text layer of the `.pdf` in `document`.
///
/// # Errors
///
/// [`DocumentCallError`]. A document that parses but carries no text layer — a
/// scan — is not an error: it yields an empty string.
pub async fn extract_text(config: &Config, document: &[u8]) -> Result<String, DocumentCallError> {
    let (runtime, record) = ready(config).await?;
    let proxy = proxy(runtime, record)?;
    let (destination, path, interface) = address(record)?;

    let handle: OutputRef = runtime
        .connection()
        .call_with_stream(
            destination,
            path,
            interface,
            member(methods::EXTRACT_TEXT)?,
            |stream| serde_json::json!([stream]),
            document,
        )
        .await
        .map_err(|error| classify(&error))?;

    let bytes = collect(&proxy, handle).await?;
    String::from_utf8(bytes)
        .map_err(|_| DocumentCallError::Failed("extracted text was not valid UTF-8".to_string()))
}

/// Load the document module if it is not already serving.
///
/// Callers do not have to invoke this — every operation below does it — but a
/// caller that wraps its work in a deadline should, *outside* that deadline.
/// A first use may download and verify an artifact, and charging that against a
/// generation timeout means the first document a user ever asks for is the one
/// that fails. Every later call finds it cached and returns immediately.
///
/// # Errors
///
/// The same [`DocumentCallError::Unavailable`] the operations return.
pub async fn ensure_ready(config: &Config) -> Result<(), DocumentCallError> {
    ops::ensure_loaded(config, MODULE_ID)
        .await
        .map_err(DocumentCallError::Unavailable)
}

/// Ensure the module is serving and hand back what a call needs.
async fn ready(
    config: &Config,
) -> Result<(&'static host::ModuleRuntime, &'static super::ModuleRecord), DocumentCallError> {
    ops::ensure_loaded(config, MODULE_ID)
        .await
        .map_err(DocumentCallError::Unavailable)?;
    let record = registry::find(MODULE_ID)
        .ok_or_else(|| DocumentCallError::Unavailable(format!("unknown module '{MODULE_ID}'")))?;
    let runtime = host::runtime()
        .await
        .map_err(|_| DocumentCallError::Unavailable("the module bus is not running".to_string()))?;
    Ok((runtime, record))
}

/// A proxy for the module's object.
fn proxy(
    runtime: &'static host::ModuleRuntime,
    record: &super::ModuleRecord,
) -> Result<tinybus::Proxy, DocumentCallError> {
    runtime
        .proxy(record.bus_name, record.object_path)
        .map_err(|error| DocumentCallError::Failed(error.to_string()))
}

/// The destination triple a streaming call needs.
fn address(
    record: &super::ModuleRecord,
) -> Result<
    (
        tinybus::BusName,
        tinybus::ObjectPath,
        tinybus::InterfaceName,
    ),
    DocumentCallError,
> {
    let bad = |error: tinybus::Error| DocumentCallError::Failed(error.to_string());
    Ok((
        tinybus::BusName::new(record.bus_name).map_err(bad)?,
        tinybus::ObjectPath::new(record.object_path).map_err(bad)?,
        tinybus::InterfaceName::new(record.bus_name).map_err(bad)?,
    ))
}

/// A member name, which is a constant in every caller here.
fn member(name: &str) -> Result<tinybus::MemberName, DocumentCallError> {
    tinybus::MemberName::new(name).map_err(|error| DocumentCallError::Failed(error.to_string()))
}

/// Pull a held document, verify it, and release it.
///
/// The release runs whether or not the read succeeded: a document left behind
/// costs the module's budget until its TTL expires, and the next caller sees a
/// full store rather than a slot.
async fn collect(proxy: &tinybus::Proxy, handle: OutputRef) -> Result<Vec<u8>, DocumentCallError> {
    let result = read_all(proxy, &handle).await;
    if let Err(error) = proxy
        .call::<()>(methods::RELEASE_OUTPUT, (handle.output_id.clone(),))
        .await
    {
        // Not fatal: the module expires what nobody reads. Worth a line, because
        // a pattern of these means documents are sitting in the module's budget
        // until they time out.
        log::debug!("[modules] releasing a read document failed: {error}");
    }
    result
}

/// Read a held document in chunks and check it against its digest.
async fn read_all(
    proxy: &tinybus::Proxy,
    handle: &OutputRef,
) -> Result<Vec<u8>, DocumentCallError> {
    if handle.total_bytes > MAX_DOCUMENT_BYTES {
        return Err(DocumentCallError::Failed(format!(
            "the module declared a {}-byte document, over the {MAX_DOCUMENT_BYTES}-byte limit",
            handle.total_bytes
        )));
    }
    // Reserve against the declared length only once it is known to be sane, and
    // still only up to one chunk beyond what has arrived — the loop grows the
    // buffer as real bytes land rather than trusting the number up front.
    let capacity = usize::try_from(handle.total_bytes).unwrap_or(0);
    let mut out = Vec::with_capacity(capacity.min(MAX_DOCUMENT_BYTES as usize));
    while (out.len() as u64) < handle.total_bytes {
        let encoded: String = proxy
            .call(
                methods::READ_OUTPUT,
                (handle.output_id.clone(), out.len() as u64, READ_CHUNK),
            )
            .await
            .map_err(|error| classify(&error))?;
        let chunk = BASE64.decode(encoded).map_err(|_| {
            DocumentCallError::Failed("a document chunk was not base64".to_string())
        })?;
        if chunk.is_empty() {
            // The module clamps a read to what is left, so an empty chunk before
            // the declared length means the two sides disagree about the size.
            // Looping would spin forever.
            return Err(DocumentCallError::Failed(
                "the document ended before its declared length".to_string(),
            ));
        }
        out.extend_from_slice(&chunk);
    }

    let digest = sha256_hex(&out);
    if digest != handle.sha256 {
        return Err(DocumentCallError::Failed(
            "the assembled document did not match its declared digest".to_string(),
        ));
    }
    Ok(out)
}

/// Lowercase hex SHA-256, matching what the module declares.
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Map a bus failure onto the shape a tool can act on.
///
/// The wire name is the contract; the message is for a human. An unrecognised
/// name is `Failed` rather than `InvalidInput`, because telling a model its input
/// was wrong when it was not sends it into a rewrite loop.
fn classify(error: &tinybus::Error) -> DocumentCallError {
    let message = error.to_string();
    match error.wire_name() {
        "ai.tinyhumans.tinydocs.Error.InvalidInput" => DocumentCallError::InvalidInput(message),
        // The module is loaded but not answering: refused, faulted, or gone.
        name if name.contains("ModuleUnavailable") => DocumentCallError::Unavailable(message),
        _ => DocumentCallError::Failed(message),
    }
}

#[cfg(test)]
#[path = "documents_tests.rs"]
mod tests;
