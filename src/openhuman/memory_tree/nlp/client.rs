//! Long-lived stdio client for the spaCy NER service.
//!
//! Spawns `service.py` under the provisioned venv interpreter, reads the one
//! `{"ready": true}` handshake line, then issues one request per query over a
//! mutex-guarded stdin/stdout pair. The model loads once for the life of the
//! process; queries are cheap line round-trips.
//!
//! A process-global [`OnceCell`] memoises the (possibly failed) initialisation
//! so the expensive provisioning + model load happens at most once. If init
//! fails (no Python, spaCy install failed, model load error) the cell stores
//! `None` and every caller falls back to the in-Rust extractor for the rest of
//! the process lifetime.

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{Mutex, OnceCell};

use crate::openhuman::config::Config;
use crate::openhuman::memory_tree::nlp::provision::{ensure_spacy, SpacyRuntime};

/// Per-request timeout for a single extraction round-trip. The model is
/// already loaded; extraction of a short query is sub-100ms, so this is just a
/// guard against a wedged child.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// One named entity span as reported by spaCy.
#[derive(Debug, Clone, Deserialize)]
pub struct SpacyEntity {
    pub text: String,
    pub label: String,
    #[serde(default)]
    pub start: u32,
    #[serde(default)]
    pub end: u32,
}

/// Parsed extraction response for one query.
#[derive(Debug, Clone, Deserialize)]
pub struct SpacyResponse {
    #[serde(default)]
    pub entities: Vec<SpacyEntity>,
    #[serde(default)]
    pub nouns: Vec<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Deserialize)]
struct ReadyLine {
    #[serde(default)]
    ready: bool,
    #[serde(default)]
    error: Option<String>,
}

/// Mutable I/O state for the child, guarded by a mutex so requests serialise.
struct Inner {
    // `_child` is retained so the process stays alive (and is killed on drop).
    _child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

/// Handle to a running spaCy NER service.
pub struct SpacyNer {
    inner: Mutex<Inner>,
}

impl SpacyNer {
    /// Spawn the service and complete the readiness handshake.
    async fn spawn(runtime: &SpacyRuntime) -> Result<Self> {
        log::debug!(
            "[memory_tree::nlp] spawning spaCy service python={} script={}",
            runtime.python_bin.display(),
            runtime.service_script.display()
        );
        let mut child = Command::new(&runtime.python_bin)
            .arg("-u")
            .arg(&runtime.service_script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| "spawning spaCy service process")?;

        let stdin = child.stdin.take().context("spaCy child stdin missing")?;
        let stdout = child.stdout.take().context("spaCy child stdout missing")?;
        let mut lines = BufReader::new(stdout).lines();

        // Handshake: wait for the ready line.
        let ready_line =
            match tokio::time::timeout(Duration::from_secs(30), lines.next_line()).await {
                Ok(Ok(Some(line))) => line,
                Ok(Ok(None)) => bail!("spaCy service exited before readiness handshake"),
                Ok(Err(e)) => return Err(e).context("reading spaCy readiness line"),
                Err(_) => bail!("spaCy service readiness handshake timed out"),
            };
        let ready: ReadyLine = serde_json::from_str(&ready_line)
            .with_context(|| format!("parsing spaCy ready line: {ready_line}"))?;
        if !ready.ready {
            bail!(
                "spaCy service failed to load model: {}",
                ready.error.unwrap_or_else(|| "unknown".into())
            );
        }

        log::info!("[memory_tree::nlp] spaCy service ready");
        Ok(Self {
            inner: Mutex::new(Inner {
                _child: child,
                stdin,
                stdout: lines,
                next_id: 0,
            }),
        })
    }

    /// Extract named entities + salient nouns from `text`.
    pub async fn extract(&self, text: &str) -> Result<SpacyResponse> {
        let mut guard = self.inner.lock().await;
        let id = guard.next_id;
        guard.next_id += 1;
        let id_str = id.to_string();

        let req = serde_json::json!({ "id": id_str, "text": text });
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        guard
            .stdin
            .write_all(line.as_bytes())
            .await
            .context("writing spaCy request")?;
        guard
            .stdin
            .flush()
            .await
            .context("flushing spaCy request")?;

        // Read until the response matching our id (skip stray lines).
        loop {
            let next = tokio::time::timeout(REQUEST_TIMEOUT, guard.stdout.next_line()).await;
            let line = match next {
                Ok(Ok(Some(l))) => l,
                Ok(Ok(None)) => bail!("spaCy service closed mid-request"),
                Ok(Err(e)) => return Err(e).context("reading spaCy response"),
                Err(_) => bail!("spaCy request timed out"),
            };
            let resp: SpacyResponse = match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(e) => {
                    log::warn!("[memory_tree::nlp] unparseable spaCy line skipped: {e}");
                    continue;
                }
            };
            if resp.id.as_deref() == Some(id_str.as_str()) {
                if let Some(err) = &resp.error {
                    bail!("spaCy extraction error: {err}");
                }
                return Ok(resp);
            }
            // Different id — keep reading.
        }
    }
}

/// Process-global memoised NER handle. `None` means initialisation was
/// attempted and failed; callers fall back to the Rust extractor.
static SPACY: OnceCell<Option<Arc<SpacyNer>>> = OnceCell::const_new();

/// Get the shared spaCy NER handle, initialising (provision + spawn) on first
/// call. Returns `None` when spaCy is unavailable; never errors so callers can
/// branch cleanly to the fallback path.
pub async fn shared_ner(config: &Config) -> Option<Arc<SpacyNer>> {
    SPACY
        .get_or_init(|| async {
            match init_ner(config).await {
                Ok(ner) => Some(Arc::new(ner)),
                Err(e) => {
                    log::warn!("[memory_tree::nlp] spaCy unavailable, using Rust fallback: {e:#}");
                    None
                }
            }
        })
        .await
        .clone()
}

async fn init_ner(config: &Config) -> Result<SpacyNer> {
    let runtime = ensure_spacy(config).await?;
    SpacyNer::spawn(&runtime).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_line_parses() {
        let r: ReadyLine =
            serde_json::from_str(r#"{"ready":true,"model":"en_core_web_sm"}"#).unwrap();
        assert!(r.ready);
        let bad: ReadyLine = serde_json::from_str(r#"{"ready":false,"error":"no spacy"}"#).unwrap();
        assert!(!bad.ready);
        assert_eq!(bad.error.as_deref(), Some("no spacy"));
    }

    #[test]
    fn response_parses_entities_and_nouns() {
        let resp: SpacyResponse = serde_json::from_str(
            r#"{"id":"0","entities":[{"text":"Alice","label":"PERSON","start":0,"end":5}],"nouns":["migration","runbook"]}"#,
        )
        .unwrap();
        assert_eq!(resp.entities.len(), 1);
        assert_eq!(resp.entities[0].label, "PERSON");
        assert_eq!(resp.nouns, vec!["migration", "runbook"]);
    }
}
