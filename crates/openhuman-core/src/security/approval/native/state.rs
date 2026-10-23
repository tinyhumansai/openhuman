//! What the native engine keeps in memory between a gated call and its decision.
//!
//! The module calls back into the host (store, audit, prompt, human
//! verification) from its own tasks, where no per-call context exists, so the
//! host state those callbacks consult is process-wide: the registered subjects
//! by their module handle, the calls awaiting a prompt by their correlation, the
//! requests the module made durable, and the human tickets not yet verified.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Instant;

use tinysecurity_bus::{ApprovalChoice2, ApprovalRow2, EvaluateRequest, SubjectScope};

use super::super::engine::PromptSurface;
use super::scopes::ScopeEntry;

/// Requests and calls kept at most; the oldest is dropped past it.
const CAPACITY: usize = 4096;

/// How long a human ticket stays valid for the module to verify.
const TICKET_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// A gated call waiting for the module to ask for its prompt.
pub(super) struct CallEntry {
    pub(super) prompt: Arc<dyn PromptSurface>,
    pub(super) scope: Arc<ScopeEntry>,
    pub(super) request: EvaluateRequest,
}

/// A request the module made durable in this process.
#[derive(Clone)]
pub(super) struct RequestEntry {
    pub(super) scope: Arc<ScopeEntry>,
    /// The exact original operation, rehashed by the module at every phase.
    pub(super) request: EvaluateRequest,
    /// The pending row as it was shown to the user; a human decision must match
    /// every byte of it.
    pub(super) delivered: Option<ApprovalRow2>,
    created: Instant,
}

impl RequestEntry {
    pub(super) fn new(
        scope: Arc<ScopeEntry>,
        request: EvaluateRequest,
        delivered: Option<ApprovalRow2>,
    ) -> Self {
        Self {
            scope,
            request,
            delivered,
            created: Instant::now(),
        }
    }
}

/// A human decision the host authenticated, until the module verifies it.
pub(super) struct Ticket {
    pub(super) request_id: String,
    pub(super) choice: ApprovalChoice2,
    pub(super) actor: String,
    issued: Instant,
}

impl Ticket {
    pub(super) fn new(request_id: String, choice: ApprovalChoice2, actor: String) -> Self {
        Self {
            request_id,
            choice,
            actor,
            issued: Instant::now(),
        }
    }
}

/// The registered subjects.
#[derive(Default)]
pub(super) struct Scopes {
    /// By `(user, workspace, agent)` of the original subject.
    pub(super) by_subject: HashMap<String, Arc<ScopeEntry>>,
    /// By the module's handle for the subject, which its store and prompt
    /// callbacks carry.
    pub(super) by_handle: HashMap<(String, String, String), Arc<ScopeEntry>>,
    /// By the audit stream id the host chose for the subject. The sink callback
    /// carries the subject only as a lossy identifier hash, so the stream id,
    /// which is unique to one subject, is what names it.
    pub(super) by_stream: HashMap<String, Arc<ScopeEntry>>,
}

/// The process-wide host state.
pub(super) struct Host {
    pub(super) scopes: tokio::sync::Mutex<Scopes>,
    calls: Mutex<HashMap<String, Arc<CallEntry>>>,
    requests: Mutex<HashMap<String, RequestEntry>>,
    tickets: Mutex<HashMap<String, Ticket>>,
}

/// The process-wide host state.
pub(super) fn host() -> &'static Host {
    static HOST: OnceLock<Host> = OnceLock::new();
    HOST.get_or_init(|| Host {
        scopes: tokio::sync::Mutex::new(Scopes::default()),
        calls: Mutex::new(HashMap::new()),
        requests: Mutex::new(HashMap::new()),
        tickets: Mutex::new(HashMap::new()),
    })
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn handle_key(handle: &SubjectScope) -> (String, String, String) {
    (
        handle.user.clone(),
        handle.workspace.clone(),
        handle.agent.clone(),
    )
}

impl Host {
    /// Remember a call until the module has asked for its prompt. `false` when
    /// the correlation is already in use.
    pub(super) fn begin_call(&self, call_id: &str, entry: CallEntry) -> bool {
        let mut calls = lock(&self.calls);
        if calls.contains_key(call_id) {
            return false;
        }
        if calls.len() >= CAPACITY {
            tracing::warn!("[approval::native] too many calls awaiting a prompt");
            return false;
        }
        calls.insert(call_id.to_owned(), Arc::new(entry));
        true
    }

    pub(super) fn call(&self, call_id: &str) -> Option<Arc<CallEntry>> {
        lock(&self.calls).get(call_id).cloned()
    }

    pub(super) fn end_call(&self, call_id: &str) {
        lock(&self.calls).remove(call_id);
    }

    pub(super) fn insert_request(&self, request_id: &str, entry: RequestEntry) {
        let mut requests = lock(&self.requests);
        if requests.len() >= CAPACITY && !requests.contains_key(request_id) {
            let oldest = requests
                .iter()
                .min_by_key(|(_, entry)| entry.created)
                .map(|(id, _)| id.clone());
            if let Some(oldest) = oldest {
                tracing::warn!("[approval::native] dropping the oldest remembered request");
                requests.remove(&oldest);
            }
        }
        requests.insert(request_id.to_owned(), entry);
    }

    pub(super) fn request(&self, request_id: &str) -> Option<RequestEntry> {
        lock(&self.requests).get(request_id).cloned()
    }

    pub(super) fn forget_request(&self, request_id: &str) {
        lock(&self.requests).remove(request_id);
    }

    pub(super) fn issue(&self, ticket: String, entry: Ticket) {
        let mut tickets = lock(&self.tickets);
        tickets.retain(|_, held| held.issued.elapsed() < TICKET_TTL);
        tickets.insert(ticket, entry);
    }

    /// The ticket's decision, without consuming it: the module verifies the same
    /// ticket again when it retries an indeterminate decision.
    pub(super) fn ticket(&self, ticket: &str) -> Option<(String, ApprovalChoice2, String)> {
        lock(&self.tickets)
            .get(ticket)
            .filter(|held| held.issued.elapsed() < TICKET_TTL)
            .map(|held| (held.request_id.clone(), held.choice, held.actor.clone()))
    }

    pub(super) fn revoke(&self, ticket: &str) {
        lock(&self.tickets).remove(ticket);
    }
}
