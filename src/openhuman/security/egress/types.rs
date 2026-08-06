//! Egress descriptor types — the privacy epic's "egress spine" (S2, #4436).
//!
//! An [`EgressDescriptor`] is the single, uniform answer to the question every
//! external transfer must carry: **what** leaves the device, **to where**, and
//! **why**. Every external-egress point in the core (LLM inference, Composio
//! tool calls, backend integrations, network-fetch tools, cloud embeddings)
//! constructs one and hands it to
//! [`emit_external_transfer`](super::emit::emit_external_transfer), which
//! publishes a [`DomainEvent::ExternalTransferPending`] before the transfer so
//! downstream slices can disclose (S3), approve (S4), and enforce (S7) it.
//!
//! ## Forward compatibility with the identification-risk detector (S5, #4439)
//!
//! The descriptor already carries the two risk fields the S5 PII /
//! identification-risk detector will populate — [`EgressDescriptor::risk_level`]
//! and [`EgressDescriptor::risk_categories`]. They default to
//! [`IdentificationRisk::Unknown`] / empty here so S5 can fill them via
//! [`EgressDescriptor::with_risk`] without a breaking reshape of this struct or
//! the [`DomainEvent`](crate::core::event_bus::DomainEvent) variant that carries
//! it.

use serde::{Deserialize, Serialize};

/// The category of data crossing the egress boundary — the "what leaves" axis.
///
/// A single transfer may carry several kinds (e.g. a tool call that ships both
/// arguments and an attached file), hence [`EgressDescriptor::data_kinds`] is a
/// list. Kinds are intentionally coarse: they classify the *shape* of the
/// payload for disclosure, never its contents (the raw bytes never travel on
/// the descriptor or its event).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataKind {
    /// LLM prompt / conversation content sent for inference.
    Prompt,
    /// Structured arguments of a tool / action call.
    ToolArguments,
    /// Text submitted to an embedding model.
    EmbeddingInput,
    /// File bytes / multipart upload content.
    FileContent,
    /// A URL / host the agent is contacting (the destination itself is data).
    Url,
    /// Non-content metadata (ids, counts, routing fields).
    Metadata,
}

/// Why the transfer is happening — the "why" axis of the descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EgressReason {
    /// LLM chat/completion inference.
    Inference,
    /// A tool / action call to a third-party provider (e.g. Composio).
    ToolCall,
    /// A round-trip to the OpenHuman managed backend / integrations API.
    Integration,
    /// Text sent to a cloud embedding provider.
    Embedding,
    /// An agent-driven network fetch (http_request / web_fetch / curl tools).
    NetworkFetch,
}

/// Identification-risk level for the payload — **populated by the S5 detector
/// (#4439)**, defaulted to [`IdentificationRisk::Unknown`] by S2.
///
/// Ordered least → most identifying. S2 never sets anything other than
/// `Unknown`; the enum lives here now so the descriptor's shape (and the
/// [`DomainEvent`](crate::core::event_bus::DomainEvent) that carries it) is
/// stable before S5 wires the detector in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentificationRisk {
    /// Not yet assessed (the S5 detector has not run for this transfer).
    #[default]
    Unknown,
    /// Assessed — no identifying content detected.
    None,
    /// Low identification risk.
    Low,
    /// Medium identification risk.
    Medium,
    /// High identification risk.
    High,
}

/// Uniform metadata describing a single external data transfer.
///
/// See the [module docs](self) for the role this plays in the privacy epic.
/// Construct one via a semantic constructor ([`EgressDescriptor::inference`],
/// [`EgressDescriptor::composio`], …) rather than the raw struct literal so the
/// per-site `reason` / `data_kinds` stay consistent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EgressDescriptor {
    /// Short, stable provider slug — the "to where" (e.g. `"openai"`,
    /// `"composio"`, `"openhuman_backend"`, `"network"`).
    pub provider_slug: String,
    /// Specific service / endpoint within the provider (e.g. a model id, a
    /// toolkit/tool slug, a backend path, or a destination host).
    pub service: String,
    /// Whether this transfer actually leaves the device. `false` for
    /// local-only runtimes (Ollama / LM Studio / MLX / local-openai) — those
    /// never publish an `ExternalTransferPending` event.
    pub is_external: bool,
    /// Why the transfer is happening.
    pub reason: EgressReason,
    /// What categories of data cross the boundary.
    pub data_kinds: Vec<DataKind>,

    // ── S5 (#4439) identification-risk fields — default-empty in S2 ──────────
    /// Identification-risk level. Populated by the S5 detector; `Unknown` until
    /// then. Serialized with `#[serde(default)]` so events emitted by S2 (which
    /// omit it) and any persisted form round-trip cleanly once S5 lands.
    #[serde(default)]
    pub risk_level: IdentificationRisk,
    /// Matched identification-risk categories (e.g. `"email"`, `"phone"`,
    /// `"credential"`). Empty until the S5 detector populates it.
    #[serde(default)]
    pub risk_categories: Vec<String>,
}

impl EgressDescriptor {
    /// Base constructor. Prefer the semantic constructors below; use this only
    /// for a site whose shape none of them capture.
    pub fn new(
        provider_slug: impl Into<String>,
        service: impl Into<String>,
        is_external: bool,
        reason: EgressReason,
        data_kinds: Vec<DataKind>,
    ) -> Self {
        Self {
            provider_slug: provider_slug.into(),
            service: service.into(),
            is_external,
            reason,
            data_kinds,
            risk_level: IdentificationRisk::Unknown,
            risk_categories: Vec::new(),
        }
    }

    /// LLM inference egress. `is_external` distinguishes a cloud provider from a
    /// local runtime (Ollama/LM Studio/etc.) — local inference is disclosed as
    /// non-external and never fires the pending event.
    pub fn inference(
        provider_slug: impl Into<String>,
        model: impl Into<String>,
        is_external: bool,
    ) -> Self {
        Self::new(
            provider_slug,
            model,
            is_external,
            EgressReason::Inference,
            vec![DataKind::Prompt],
        )
    }

    /// Composio tool-call egress (always external).
    pub fn composio(tool: impl Into<String>) -> Self {
        Self::new(
            "composio",
            tool,
            true,
            EgressReason::ToolCall,
            vec![DataKind::ToolArguments],
        )
    }

    /// OpenHuman managed-backend / integrations round-trip (always external —
    /// the backend is off-device). `service` is the request path.
    pub fn integration(service: impl Into<String>) -> Self {
        Self::new(
            "openhuman_backend",
            service,
            true,
            EgressReason::Integration,
            vec![DataKind::Metadata],
        )
    }

    /// Cloud embedding egress (always external). `provider_slug` is the
    /// embedding provider, `model` the embedding model id.
    pub fn embedding(provider_slug: impl Into<String>, model: impl Into<String>) -> Self {
        Self::new(
            provider_slug,
            model,
            true,
            EgressReason::Embedding,
            vec![DataKind::EmbeddingInput],
        )
    }

    /// Agent network-fetch tool egress (http_request / web_fetch / curl). The
    /// destination host is the payload's most identifying field, so `service`
    /// is the host and `data_kinds` records that a URL is being contacted.
    pub fn network_fetch(host: impl Into<String>) -> Self {
        Self::new(
            "network",
            host,
            true,
            EgressReason::NetworkFetch,
            vec![DataKind::Url],
        )
    }

    /// Add a `data_kind` if not already present (for sites that carry more than
    /// the constructor's default, e.g. a network POST that also ships a body).
    pub fn with_data_kind(mut self, kind: DataKind) -> Self {
        if !self.data_kinds.contains(&kind) {
            self.data_kinds.push(kind);
        }
        self
    }

    /// **S5 (#4439) hook.** Attach the identification-risk assessment produced
    /// by the PII detector. S2 never calls this; it exists so S5 can populate
    /// the risk fields without reshaping the descriptor.
    pub fn with_risk(mut self, level: IdentificationRisk, categories: Vec<String>) -> Self {
        self.risk_level = level;
        self.risk_categories = categories;
        self
    }
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
