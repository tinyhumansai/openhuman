//! One-shot approval requests for TinySecurity keyring operations.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use tokio::sync::oneshot;

pub(crate) const MAX_PENDING_APPROVALS: usize = 64;
/// The longest the host waits for the user to answer one keyring prompt.
pub(crate) const MAX_CONSENT_WAIT_MS: u32 = 120_000;
/// How long a decided outcome waits for its caller. The caller takes it as
/// soon as the module answers; one still here is from a caller that has gone.
const OUTCOME_TTL_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OperationOutcome {
    Denied,
    Pending,
}

#[derive(Debug, Clone)]
pub(crate) struct ApprovalPrompt {
    pub request_id: String,
    pub access: String,
    pub name: String,
    pub expires_at_ms: u64,
}

#[derive(Default)]
pub(crate) struct PendingConsentRegistry {
    pending: HashMap<String, oneshot::Sender<bool>>,
    reserved_call_ids: HashSet<String>,
    outcomes: HashMap<String, (OperationOutcome, u64)>,
    /// The prompt each pending request shows, for a renderer that cannot
    /// receive the approval event (no socket before sign-in).
    prompts: HashMap<String, crate::core::events::KeyringAccessApprovalRequest>,
}

static PENDING: OnceLock<Mutex<PendingConsentRegistry>> = OnceLock::new();

fn pending() -> &'static Mutex<PendingConsentRegistry> {
    PENDING.get_or_init(|| Mutex::new(PendingConsentRegistry::default()))
}

impl PendingConsentRegistry {
    pub(crate) fn register(
        &mut self,
        access: &str,
        name: &str,
        call_id: &str,
        timeout: std::time::Duration,
    ) -> Result<(ApprovalPrompt, oneshot::Receiver<bool>), &'static str> {
        let now = now_ms();
        self.outcomes
            .retain(|_, (_, decided_at)| now.saturating_sub(*decided_at) < OUTCOME_TTL_MS);
        if self.reserved_call_ids.len() + self.outcomes.len() >= MAX_PENDING_APPROVALS {
            return Err("too many pending keyring consent requests");
        }
        if self.reserved_call_ids.contains(call_id) || self.outcomes.contains_key(call_id) {
            return Err("duplicate keyring operation id");
        }

        let (sender, receiver) = oneshot::channel();
        let request_id = uuid::Uuid::new_v4().to_string();
        self.pending.insert(request_id.clone(), sender);
        self.reserved_call_ids.insert(call_id.to_owned());
        Ok((
            ApprovalPrompt {
                request_id,
                access: access.to_owned(),
                name: name.to_owned(),
                expires_at_ms: now
                    .saturating_add(timeout.as_millis().try_into().unwrap_or(u64::MAX)),
            },
            receiver,
        ))
    }

    pub(crate) fn resolve(&mut self, request_id: &str, approved: bool) -> bool {
        self.prompts.remove(request_id);
        self.pending
            .remove(request_id)
            .is_some_and(|sender| sender.send(approved).is_ok())
    }

    pub(crate) fn cancel(&mut self, request_id: &str) {
        self.pending.remove(request_id);
        self.prompts.remove(request_id);
    }

    /// The prompts still waiting for a decision at `now_ms`.
    pub(crate) fn pending_prompts(
        &self,
        now_ms: u64,
    ) -> Vec<crate::core::events::KeyringAccessApprovalRequest> {
        let mut prompts: Vec<_> = self
            .prompts
            .values()
            .filter(|prompt| {
                prompt.expires_at_ms > now_ms && self.pending.contains_key(&prompt.request_id)
            })
            .cloned()
            .collect();
        prompts.sort_by_key(|prompt| prompt.expires_at_ms);
        prompts
    }

    fn finish(&mut self, request_id: &str, call_id: &str, outcome: Option<OperationOutcome>) {
        self.pending.remove(request_id);
        self.prompts.remove(request_id);
        if self.reserved_call_ids.remove(call_id) {
            if let Some(outcome) = outcome {
                self.outcomes
                    .insert(call_id.to_owned(), (outcome, now_ms()));
            }
        }
    }

    fn take_operation_outcome(&mut self, call_id: &str) -> Option<OperationOutcome> {
        self.outcomes.remove(call_id).map(|(outcome, _)| outcome)
    }
}

/// Request approval for this exact operation and wait for its correlated
/// response. Missing event infrastructure, a full queue, timeout, or a lost
/// waiter all fail closed.
pub(crate) async fn request_approval(
    request: tinysecurity_bus::KeyringConsentRequest2,
) -> Option<bool> {
    use tinysecurity_bus::KeyringAccess2;

    let access = match request.access {
        KeyringAccess2::Read => "read",
        KeyringAccess2::Write => "write",
        KeyringAccess2::Delete => "delete",
    };
    let timeout =
        std::time::Duration::from_millis(u64::from(request.timeout_ms.min(MAX_CONSENT_WAIT_MS)));
    let call_id = request.caller.call_id;
    let (prompt, waiter, shown) = {
        let mut registry = pending().lock().ok()?;
        let (prompt, waiter) = registry
            .register(access, &request.name, &call_id, timeout)
            .ok()?;
        let shown = crate::core::events::KeyringAccessApprovalRequest {
            request_id: prompt.request_id.clone(),
            caller_agent: request.caller.agent.clone(),
            origin: format!("{:?}", request.caller.origin),
            purpose: request.purpose,
            access: prompt.access.clone(),
            name: prompt.name.clone(),
            expires_at_ms: prompt.expires_at_ms,
        };
        registry
            .prompts
            .insert(prompt.request_id.clone(), shown.clone());
        (prompt, waiter, shown)
    };
    let mut reservation = Reservation {
        request_id: prompt.request_id.clone(),
        call_id,
        outcome: None,
    };
    crate::core::bus::BUS
        .publish(crate::core::events::DomainEvent::KeyringAccessApprovalRequested(shown));
    #[cfg(any(test, feature = "e2e-test-support", feature = "test-keyring-consent"))]
    if let Some(approved) = super::test_responder::decision(&request.caller.agent) {
        resolve_approval(&prompt.request_id, approved);
    }

    let decision = await_decision(pending(), &prompt.request_id, waiter, timeout).await;
    reservation.outcome = match decision {
        Some(false) => Some(OperationOutcome::Denied),
        None => Some(OperationOutcome::Pending),
        Some(true) => None,
    };
    drop(reservation);
    decision
}

/// One registered operation, finished when dropped: with its outcome once
/// decided, or with none if the waiting caller goes away first.
struct Reservation {
    request_id: String,
    call_id: String,
    outcome: Option<OperationOutcome>,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Ok(mut registry) = pending().lock() {
            registry.finish(&self.request_id, &self.call_id, self.outcome);
        }
    }
}

/// Consume the decision for one host-generated keyring operation id.
pub(crate) fn take_operation_outcome(call_id: &str) -> Option<OperationOutcome> {
    pending()
        .lock()
        .ok()
        .and_then(|mut registry| registry.take_operation_outcome(call_id))
}

async fn await_decision(
    registry: &Mutex<PendingConsentRegistry>,
    request_id: &str,
    waiter: oneshot::Receiver<bool>,
    timeout: std::time::Duration,
) -> Option<bool> {
    match tokio::time::timeout(timeout, waiter).await {
        Ok(Ok(decision)) => Some(decision),
        Ok(Err(_)) | Err(_) => {
            if let Ok(mut pending) = registry.lock() {
                pending.cancel(request_id);
            }
            None
        }
    }
}

/// The keyring operations waiting for a decision. Display-safe metadata only,
/// the same the approval event carries; never a secret value.
pub(crate) fn pending_requests() -> Vec<crate::core::events::KeyringAccessApprovalRequest> {
    pending()
        .lock()
        .map(|registry| registry.pending_prompts(now_ms()))
        .unwrap_or_default()
}

/// Resolve one currently pending request. Each id can be consumed once.
pub(crate) fn resolve_approval(request_id: &str, approved: bool) -> bool {
    pending()
        .lock()
        .is_ok_and(|mut pending| pending.resolve(request_id, approved))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "operation_approval_tests.rs"]
mod tests;
