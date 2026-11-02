//! What `[inbound]` resolves to: the queue manifest and pipeline configuration
//! the module runs, where its queues live, and whether routing is on.
//!
//! The host hands the module its queue and pipeline configuration in the
//! module's private blob when it loads it ([`module_config`]), and declares the
//! same manifest on the module's storage scope when the storage host binds it
//! ([`queue_grant`]). TinyBus injects the unique storage callback destination
//! during verified module loading. Both read the last resolution, so the queues
//! the module asks for are exactly the queues the host serves.

use std::path::Path;
use std::sync::{Arc, RwLock};

use serde_json::{json, Value};
use tinyeventqueues_bus::storage::{SHIPPED_PIPELINE_JSON, SHIPPED_QUEUES_JSON};
use tinyeventqueues_bus::{Lane, PipelineConfig};
use tinystoragedrivers::QueueConfig;

use crate::config::schema::InboundConfig;
use crate::config::Config;
use crate::modules::storage_grant_schemas::QueueGrant;
use crate::modules::storage_queue_manifest::QueueManifest;

/// The `[components]` switch that owns the inbound event queues.
pub const COMPONENT: &str = "eventqueues";

/// Whether inbound events are routed through the queues under `config`:
/// modules on, the `eventqueues` component on, and `[inbound] enabled`.
#[must_use]
pub fn active(config: &Config) -> bool {
    config.modules.enabled && config.components.eventqueues && config.inbound.enabled
}

/// One resolution of `[inbound]`.
#[derive(Debug, Clone)]
pub struct Resolved {
    /// The queues the module runs and the host declares.
    pub manifest: Arc<QueueManifest>,
    /// The manifest as the module receives it.
    pub queues_json: Value,
    /// The pipeline configuration.
    pub pipeline: PipelineConfig,
    /// The pipeline configuration as the module receives it.
    pub pipeline_json: Value,
    /// Where the queues live.
    pub backend: QueueConfig,
}

impl Resolved {
    /// The `max_attempts` of the queue `lane` dispatches from, when known.
    #[must_use]
    pub fn max_attempts(&self, lane: Lane) -> Option<u32> {
        let queue = &self.pipeline.lanes.get(lane).queue;
        self.manifest
            .get(queue)
            .map(|declared| declared.max_attempts)
    }
}

/// The shipped files with the queues in storage: what runs when `[inbound]`
/// names nothing, or names something unusable.
///
/// # Panics
///
/// Never in practice: the shipped files are the module's own and parse (the
/// tests pin it).
#[must_use]
pub fn shipped() -> Resolved {
    #[allow(clippy::expect_used)]
    resolve_texts(SHIPPED_QUEUES_JSON, SHIPPED_PIPELINE_JSON, None)
        .expect("the shipped queues.json and pipeline.json parse")
}

/// Resolve `[inbound]` under `config`.
///
/// # Errors
///
/// A file that cannot be read, a manifest or pipeline that does not parse, or
/// a queue URL that is not `storage`, `amqp://…` or `amqps://…`.
pub fn resolve(config: &Config) -> Result<Resolved, String> {
    let queues = read_or(config.inbound.queues_path.as_deref(), SHIPPED_QUEUES_JSON)?;
    let pipeline = read_or(
        config.inbound.pipeline_path.as_deref(),
        SHIPPED_PIPELINE_JSON,
    )?;
    let pipeline: Value =
        serde_json::from_str(&pipeline).map_err(|error| format!("pipeline.json: {error}"))?;
    let pipeline = with_sources(pipeline, &config.inbound).to_string();
    resolve_texts(&queues, &pipeline, config.inbound.queue_url.as_deref())
}

/// `pipeline` with one source policy per usable `[inbound.sources.*]` entry
/// ahead of its own (the first match wins): acknowledged at ingest, weighted
/// into the interactive lane, released at once rather than on a lane's
/// cadence, and debounced with the source's own windows and batch cap.
#[must_use]
pub fn with_sources(mut pipeline: Value, inbound: &InboundConfig) -> Value {
    let policies: Vec<Value> = inbound
        .usable_sources()
        .map(|(name, source)| {
            json!({
                "match": name,
                "ack": "after_ingest",
                "weight": SOURCE_WEIGHT,
                "cadence": "instant",
                "coalesce": {
                    "debounce_ms": source.debounce_ms,
                    "max_wait_ms": source.max_wait_ms,
                    "max_events": source.max_batch,
                },
            })
        })
        .collect();
    if policies.is_empty() {
        return pipeline;
    }
    let Some(object) = pipeline.as_object_mut() else {
        return pipeline;
    };
    let mut sources = policies;
    if let Some(Value::Array(own)) = object.remove("sources") {
        sources.extend(own);
    }
    object.insert("sources".to_owned(), Value::Array(sources));
    pipeline
}

/// Salience points an external source's events get: the base 25 plus this
/// reaches the interactive lane (70), so a batch is never scheduled on the
/// background lane's cadence or folded into the digest as noise.
const SOURCE_WEIGHT: i32 = 50;

fn read_or(path: Option<&Path>, shipped: &str) -> Result<String, String> {
    match path {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display())),
        None => Ok(shipped.to_owned()),
    }
}

fn resolve_texts(queues: &str, pipeline: &str, url: Option<&str>) -> Result<Resolved, String> {
    let manifest =
        QueueManifest::from_json(queues).map_err(|error| format!("queues.json: {error}"))?;
    let queues_json: Value =
        serde_json::from_str(queues).map_err(|error| format!("queues.json: {error}"))?;
    let pipeline_json: Value =
        serde_json::from_str(pipeline).map_err(|error| format!("pipeline.json: {error}"))?;
    let parsed: PipelineConfig = serde_json::from_value(pipeline_json.clone())
        .map_err(|error| format!("pipeline.json: {error}"))?;
    let url = url.unwrap_or("storage");
    refuse_plaintext_credentials(url)?;
    let backend = QueueConfig::parse(url).map_err(|error| format!("inbound.queue_url: {error}"))?;
    Ok(Resolved {
        manifest: Arc::new(manifest),
        queues_json,
        pipeline: parsed,
        pipeline_json,
        backend,
    })
}

/// A broker URL that carries credentials must use TLS: `amqp://` sends the
/// password (and every queued event) in the clear.
fn refuse_plaintext_credentials(url: &str) -> Result<(), String> {
    let Some(rest) = url.trim().strip_prefix("amqp://") else {
        return Ok(());
    };
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.contains('@') {
        return Err(
            "inbound.queue_url: a broker URL with credentials must use amqps:// (TLS)".to_owned(),
        );
    }
    Ok(())
}

/// The last resolution, read by the storage host's grant and by `Deliver`.
static LAST_RESOLUTION: RwLock<Option<Arc<Resolved>>> = RwLock::new(None);

/// The resolution the module was last configured with, or the shipped one.
#[must_use]
pub fn current() -> Arc<Resolved> {
    static SHIPPED: std::sync::OnceLock<Arc<Resolved>> = std::sync::OnceLock::new();
    let guard = LAST_RESOLUTION
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard
        .clone()
        .unwrap_or_else(|| Arc::clone(SHIPPED.get_or_init(|| Arc::new(shipped()))))
}

fn remember(resolved: Resolved) -> Arc<Resolved> {
    let resolved = Arc::new(resolved);
    *LAST_RESOLUTION
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::clone(&resolved));
    resolved
}

/// The private configuration blob the module is loaded with. An unusable
/// `[inbound]` file is logged and the shipped files run instead, so a typo in
/// an override never takes inbound events down with it.
#[must_use]
pub fn module_config(config: &Config) -> Value {
    let resolved = match resolve(config) {
        Ok(resolved) => resolved,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "[inbound] [inbound] configuration unusable; running the shipped queues and pipeline"
            );
            shipped()
        }
    };
    tracing::debug!(
        queues = resolved.manifest.queues().len(),
        backend = %resolved.backend,
        "[inbound] configuring the tinyeventqueues module"
    );
    let resolved = remember(resolved);
    // The module keeps its queues in host storage and refuses to load without
    // the bus name that serves it; the host still derives the scope and grant
    // from the authenticated caller, never from this value.
    json!({
        "storage_callback": tinystoragedrivers_bus::STORAGE_INTERFACE.to_string(),
        "queues": resolved.queues_json,
        "pipeline": resolved.pipeline_json,
    })
}

/// The queues the storage host grants the module: the manifest it was
/// configured with and where they live.
#[must_use]
pub(crate) fn queue_grant() -> Option<QueueGrant> {
    let resolved = current();
    Some(QueueGrant {
        manifest: Arc::clone(&resolved.manifest),
        backend: resolved.backend.clone(),
    })
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
