//! The opt-in store/fetch/forget roundtrip against the selected memory engine.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tinymemory_api::{
    EngineDescriptor, Error as MemoryApiError, FetchMode, FetchRequest, ItemId, MemoryEngine,
    MemoryMeta, MetaFilter, Namespace, Reach, Segment, SegmentKind, StoreItem,
};

use crate::doctor::{ComponentId, DoctorCheck, DoctorCtx, DoctorTier, Finding};
use crate::memory::engine_registry::ReadOnlyEngineError;
use crate::memory::error::MemoryError;

const CHECK_ID: &str = "memory/roundtrip";
const OPERATION_TIMEOUT: Duration = Duration::from_secs(12);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(8);

pub(super) struct MemoryRoundtripCheck;

#[async_trait]
impl DoctorCheck for MemoryRoundtripCheck {
    fn id(&self) -> &str {
        CHECK_ID
    }

    fn category(&self) -> &str {
        "memory"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Live
    }

    fn component_gate(&self) -> Option<ComponentId> {
        Some(ComponentId::Memory)
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        let config = ctx.config();
        let engine = match crate::memory::engine::resolve_read_only_for_inspection(config).await {
            Ok(engine) => engine,
            Err(ReadOnlyEngineError::EngineDisabled | ReadOnlyEngineError::ComponentDisabled) => {
                return vec![Finding::skipped(
                    CHECK_ID,
                    "Memory is disabled in the selected configuration.",
                    "Enable memory and select an engine before running the live roundtrip.",
                )];
            }
            Err(error) => return vec![finding_for_engine_error(error)],
        };

        let Some(fetch_mode) = probe_fetch_mode(engine.descriptor()) else {
            return vec![Finding::error(
                CHECK_ID,
                "The selected memory engine does not support a keyword or hybrid probe fetch.",
                "Select a memory engine that supports text-query fetch, store, and scoped forget.",
            )];
        };

        match roundtrip(engine, fetch_mode).await {
            Ok(()) => vec![Finding::pass(
                CHECK_ID,
                "The selected memory engine stored, fetched, and removed a temporary probe.",
            )],
            Err(finding) => vec![finding],
        }
    }
}

fn probe_fetch_mode(descriptor: &EngineDescriptor) -> Option<FetchMode> {
    [FetchMode::Hybrid, FetchMode::Keyword]
        .into_iter()
        .find(|mode| descriptor.supports(*mode))
}

struct ScopedScratchCleanup {
    engine: Arc<dyn MemoryEngine>,
    ids: Vec<ItemId>,
    reach: Reach,
    armed: bool,
}

impl ScopedScratchCleanup {
    fn new(engine: Arc<dyn MemoryEngine>, id: ItemId, namespace: Namespace) -> Self {
        Self {
            engine,
            ids: vec![id],
            reach: Reach::exact(namespace),
            armed: true,
        }
    }

    fn include_receipt_id(&mut self, id: ItemId) {
        if !self.ids.contains(&id) {
            self.ids.push(id);
        }
    }

    async fn cleanup(&mut self) -> bool {
        if !self.armed {
            return true;
        }
        let result = tokio::time::timeout(
            CLEANUP_TIMEOUT,
            self.engine
                .forget_within(self.ids.clone(), self.reach.clone()),
        )
        .await;
        if matches!(result, Ok(Ok(_))) {
            self.armed = false;
            true
        } else {
            false
        }
    }
}

impl Drop for ScopedScratchCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::warn!(
                check_id = CHECK_ID,
                "scratch cleanup could not be scheduled"
            );
            return;
        };
        let engine = Arc::clone(&self.engine);
        let ids = self.ids.clone();
        let reach = self.reach.clone();
        runtime.spawn(async move {
            match tokio::time::timeout(CLEANUP_TIMEOUT, engine.forget_within(ids, reach)).await {
                Ok(Ok(_)) => {}
                Ok(Err(_)) | Err(_) => {
                    tracing::warn!(
                        check_id = CHECK_ID,
                        "scratch cleanup failed after cancellation"
                    );
                }
            }
        });
    }
}

async fn roundtrip(engine: Arc<dyn MemoryEngine>, fetch_mode: FetchMode) -> Result<(), Finding> {
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let namespace = Namespace::new(vec![Segment::new(
        SegmentKind::Service,
        format!("openhuman-doctor-{nonce}"),
    )
    .expect("generated service namespace is valid")])
    .expect("one service namespace segment is valid");
    let item = StoreItem::document(
        "OpenHuman doctor memory roundtrip probe",
        MemoryMeta {
            namespace: namespace.clone(),
            ..MemoryMeta::default()
        },
    );
    // StoreItem fingerprints are the stable idempotency id used by all
    // engines. Compute it before store so an accepted-but-timed-out write can
    // still be cleaned up.
    let scratch_id = ItemId::new(item.fingerprint());
    let mut cleanup =
        ScopedScratchCleanup::new(engine.clone(), scratch_id.clone(), namespace.clone());

    let receipt = match tokio::time::timeout(OPERATION_TIMEOUT, engine.store(item)).await {
        Ok(Ok(receipt)) => receipt,
        Ok(Err(error)) => {
            let cleaned = cleanup.cleanup().await;
            return Err(failure_finding(error, Operation::Store, cleaned));
        }
        Err(_) => {
            let cleaned = cleanup.cleanup().await;
            return Err(timeout_finding(Operation::Store, cleaned));
        }
    };
    let stored_id = receipt.id;
    cleanup.include_receipt_id(stored_id.clone());

    let request = FetchRequest {
        filter: MetaFilter {
            reach: Some(Reach::exact(namespace)),
            ..MetaFilter::default()
        },
        ..FetchRequest::new("OpenHuman doctor memory roundtrip probe", fetch_mode, 1)
    };
    match tokio::time::timeout(OPERATION_TIMEOUT, engine.fetch(request)).await {
        Ok(Ok(page)) if page.hits.iter().any(|hit| hit.id == stored_id) => {}
        Ok(Ok(_)) => {
            let cleaned = cleanup.cleanup().await;
            return Err(Finding::error(
                CHECK_ID,
                "The memory engine stored the probe but could not fetch it from its scratch scope.",
                cleanup_hint(
                    "Check the selected memory engine's indexing and retry the live check.",
                    cleaned,
                ),
            ));
        }
        Ok(Err(error)) => {
            let cleaned = cleanup.cleanup().await;
            return Err(failure_finding(error, Operation::Fetch, cleaned));
        }
        Err(_) => {
            let cleaned = cleanup.cleanup().await;
            return Err(timeout_finding(Operation::Fetch, cleaned));
        }
    }

    if cleanup.cleanup().await {
        Ok(())
    } else {
        Err(Finding::error(
            CHECK_ID,
            "The memory engine answered the roundtrip but the temporary probe could not be removed.",
            "Check the selected memory engine and remove the isolated doctor service item before rerunning.",
        ))
    }
}

#[derive(Clone, Copy)]
enum Operation {
    Store,
    Fetch,
}

fn failure_finding(error: MemoryApiError, operation: Operation, cleaned: bool) -> Finding {
    // Some memory transports preserve a typed 402 as `InsufficientCredits`,
    // while others normalize it to `Engine` and retain the stable marker in
    // the sanitized display string. Recover only the known account-wide
    // refusal markers; ordinary engine errors keep the generic classification.
    let error_text = error.to_string();
    let classified = MemoryError::refusal_from_skip_reason(&error_text)
        .unwrap_or_else(|| MemoryError::from(error));
    let (message, hint) = match classified {
        MemoryError::Unauthorized(_) => (
            "The selected memory engine rejected its credential.",
            "Sign in again or update the selected memory credential, then rerun the live check.",
        ),
        MemoryError::InsufficientCredits(_) => (
            "The selected memory service has insufficient credits for a live probe.",
            "Review the TinyHumans memory billing balance before rerunning the live check.",
        ),
        MemoryError::PlanRequired(_) => (
            "Memory is not included in the selected account's plan.",
            "Upgrade to a plan that includes memory or select a different memory engine in Settings.",
        ),
        MemoryError::Unavailable(_) => (
            "The selected memory service is temporarily unavailable.",
            "Check the memory service status and retry the live check later.",
        ),
        MemoryError::Off(_) => (
            "The selected memory engine is not enabled.",
            "Enable a memory engine in the selected configuration and rerun the check.",
        ),
        MemoryError::Unsupported(_) => (
            "The selected memory engine does not support the requested probe operation.",
            "Select a memory engine that supports store, fetch, and scoped forget.",
        ),
        MemoryError::InvalidRequest(_) => (
            "The memory engine refused the doctor probe request.",
            "Check the selected memory engine configuration and rerun the live check.",
        ),
        MemoryError::Engine(_) | MemoryError::ConfigConflict(_) => (
            "The selected memory engine reported an error; details are withheld.",
            "Check the selected memory engine configuration and retry the live check.",
        ),
    };
    let operation = match operation {
        Operation::Store => "store",
        Operation::Fetch => "fetch",
    };
    Finding::error(
        CHECK_ID,
        format!("The memory {operation} probe failed. {message}"),
        cleanup_hint(hint, cleaned),
    )
}

fn timeout_finding(operation: Operation, cleaned: bool) -> Finding {
    let (name, fix) = match operation {
        Operation::Store => (
            "store",
            "Check the memory service status and retry the live check.",
        ),
        Operation::Fetch => (
            "fetch",
            "Check the memory service status and retry the live check.",
        ),
    };
    Finding::error(
        CHECK_ID,
        format!("The memory {name} probe timed out."),
        cleanup_hint(fix, cleaned),
    )
}

fn cleanup_hint(hint: &str, cleaned: bool) -> String {
    if cleaned {
        hint.to_owned()
    } else {
        format!(
            "{hint} The doctor could not confirm scratch cleanup; inspect the isolated doctor service scope."
        )
    }
}

fn finding_for_engine_error(error: ReadOnlyEngineError) -> Finding {
    match error {
        ReadOnlyEngineError::CredentialsUnavailable => Finding::error(
            CHECK_ID,
            error.safe_message(),
            "Check the selected memory credential profile and sign in again if needed.",
        ),
        ReadOnlyEngineError::EndpointUnavailable => Finding::error(
            CHECK_ID,
            error.safe_message(),
            "Configure the selected memory engine endpoint and rerun the live check.",
        ),
        ReadOnlyEngineError::EngineDisabled | ReadOnlyEngineError::ComponentDisabled => {
            Finding::skipped(
                CHECK_ID,
                error.safe_message(),
                "Enable memory and select an engine before running the live roundtrip.",
            )
        }
        ReadOnlyEngineError::UnsupportedEngine => Finding::error(
            CHECK_ID,
            error.safe_message(),
            "Select a memory engine with an inspection-safe constructor.",
        ),
        ReadOnlyEngineError::InvalidConfiguration => Finding::error(
            CHECK_ID,
            error.safe_message(),
            "Review the selected memory engine settings and rerun the live check.",
        ),
    }
}

#[cfg(test)]
#[path = "memory_live_tests.rs"]
mod tests;
