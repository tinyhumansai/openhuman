//! Composition factory for OpenHuman-owned TinyAgents sessions.

use std::sync::Arc;
use tinyagents_runtime::{
    PrefixSnapshot, RuntimeError, Session, SessionBuilder, SessionDriver, ToolSnapshot,
};
use tinyagents_session::transcript::{TranscriptLocator, TranscriptMeta};

use crate::agent::tinyagents::host::OpenHumanRunContext;

use super::{OpenHumanSessionHooks, OpenHumanTranscriptCodec};

/// Builds the neutral runtime from OpenHuman's resolved policy and adapters.
///
/// No stateful history, transcript delta or prefix implementation belongs in
/// this type; those remain directly owned by `tinyagents_runtime::Session`.
pub struct OpenHumanSessionFactory;

impl OpenHumanSessionFactory {
    pub fn build(
        driver: Arc<dyn SessionDriver<OpenHumanRunContext>>,
        hooks: Arc<OpenHumanSessionHooks>,
        prefix: PrefixSnapshot,
        tools: ToolSnapshot,
        locator: Arc<dyn TranscriptLocator>,
        stem: impl Into<String>,
        meta: TranscriptMeta,
    ) -> Result<Session<OpenHumanRunContext>, RuntimeError> {
        SessionBuilder::new(driver)
            .codec(Arc::new(OpenHumanTranscriptCodec))
            .hooks(hooks)
            .prefix(prefix)
            .tool_snapshot(tools)
            .transcript(locator, stem, meta)
            .build()
    }
}
