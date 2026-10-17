//! The host side of `ai.tinyhumans.tinystorage.Storage`: durable storage for
//! loadable modules, served from the process's installed `StorageBackend`.
//!
//! A loaded module shares the process but not the host's storage handles. It
//! calls this interface instead of keeping its own files, and every call is
//! decided here, never by the module:
//!
//! * **Who is calling.** The module is the broker-authenticated sender of the
//!   call (`CallContext::authenticated_sender`), mapped back to a registry
//!   module id by asking the broker who owns each module's bus name. A call
//!   with no authenticated sender, or from a peer that is no registry module,
//!   is refused.
//! * **Which scope.** Never read from the request, which has no such field
//!   (the wire types reject unknown fields). The scope is derived from the
//!   module id and the host's current scope ([`crate::storage::scope_from`]):
//!   `module:<id>@<base>`, so two modules never share a scope and a module can
//!   never name another tenant's. In SaaS mode a call with no profile is
//!   refused like any other storage use.
//! * **What it may touch.** The module's [`StorageGrant`], declared by the
//!   host build ([`super::storage_grants::declared`]): its collections, stream,
//!   blob and secret prefixes, and whether it may write. Anything else is a
//!   refusal before the backend is reached.
//! * **Schema.** Only the host runs `ensure_collection`, from the grant's
//!   declared specs, when it binds a module to a scope.
//! * **Fencing.** The scope handle comes from the installed backend, which the
//!   core opened wrapped in its lease fences ([`crate::storage::open`]), so a
//!   module write under a stale epoch fails `Fenced`; the module never
//!   supplies or sees a fence.
//! * **Audit.** One line per call: module id, scope, object, operation, target
//!   (a collection or a granted prefix) and outcome. Never a payload.
//!
//! The reference `StorageServer` of `tinystoragedrivers-client` answers the
//! messages once a call is allowed; this file adds only the host's decisions.
//!
//! Blobs over a bus stream (`PutStream`/`GetStream`) and watches (`WatchOpen`)
//! go through `tinystoragedrivers-busproxy`, which binds them to the peer: an
//! upload is read with `Connection::accept_stream_from` only from the
//! authenticated caller that opened it, a download is opened toward that
//! caller, and a watch's invalidations are unicast to the subscribing caller
//! with `Connection::emit_to`. Every decision above still applies first.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinybus::BusName;
use tinystoragedrivers::secrets::{DocumentSecrets, KeyProvider};
use tinystoragedrivers::BlobReader;
use tinystoragedrivers_bus::{
    BlobKeyRequest, BlobStreamHead, PutBlobStreamRequest, StorageFailure, StorageKind,
};
use tinystoragedrivers_busproxy::HostFailure;

use tinystoragedrivers_client::{Admission, StorageServer};

use self::endpoint::HostEndpoint;
use super::storage_grants::{authorize, declared, filter_reply, Object, Plan, StorageGrant};
use crate::storage::{Scope, StorageBackend, StorageError};

/// The largest blob a module may send by `PutStream`. The body is bounded per
/// chunk by the bus and by its declared length by the server, so this is the
/// host's own ceiling on one module's single upload.
const MAX_STREAMED_BLOB_BYTES: u64 = 256 * 1024 * 1024;

/// Maps a broker-authenticated sender to a registry module id.
#[async_trait]
pub(crate) trait CallerDirectory: Send + Sync {
    /// The module that owns `sender`, if it is one.
    async fn module_for(&self, sender: &BusName) -> Option<String>;
}

/// What the host decides from, separated so tests can supply their own.
#[async_trait]
pub(crate) trait Environment: Send + Sync {
    /// The installed backend, if the host configured one.
    fn backend(&self) -> Option<Arc<dyn StorageBackend>>;
    /// The scope `module_id`'s data lives in: the host's current scope with the
    /// module added ([`module_scope`]).
    fn module_scope(&self, module_id: &str) -> Result<Scope, StorageError>;
    /// The module's declared grant.
    fn grant(&self, module_id: &str) -> Option<StorageGrant>;
    /// The key provider for module secrets.
    fn keys(&self) -> Result<Arc<dyn KeyProvider>, StorageError>;
    /// Consults the host's write fence for `scope` before a write is forwarded
    /// (the backend checks it again). A refusal means the lease behind the
    /// scope is gone.
    async fn admit_write(&self, scope: &Scope) -> Result<(), StorageError>;
}

/// One line of the audit trail. Carries no payload by construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AuditRecord {
    pub module: String,
    pub scope: String,
    pub object: &'static str,
    pub op: String,
    pub target: String,
    pub outcome: &'static str,
}

/// Receives every audit record.
pub(crate) trait AuditSink: Send + Sync {
    fn record(&self, record: &AuditRecord);
}

struct LogAudit;

impl AuditSink for LogAudit {
    fn record(&self, record: &AuditRecord) {
        tracing::info!(
            target: "openhuman::storage_host",
            module = %record.module,
            scope = %record.scope,
            object = record.object,
            op = %record.op,
            target = %record.target,
            outcome = record.outcome,
            "[storage_host] module storage call"
        );
    }
}

/// How a call failed, before it becomes a bus error.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HostError {
    /// Not allowed; the label says why and holds no request content.
    Denied(&'static str),
    /// The backend refused or failed, as a content-free category.
    Storage(StorageFailure),
}

impl From<HostError> for HostFailure {
    fn from(error: HostError) -> Self {
        match error {
            HostError::Denied(label) => HostFailure::Denied(label),
            HostError::Storage(failure) => HostFailure::Storage(failure),
        }
    }
}

impl From<HostError> for tinybus::Error {
    fn from(error: HostError) -> Self {
        HostFailure::from(error).into()
    }
}

fn failure(kind: StorageKind) -> HostError {
    HostError::Storage(StorageFailure { kind })
}

/// A module's scope under the host's current tenant.
///
/// * For a profile (SaaS): `<profile>~module:<id>`, with the profile escaped
///   as a session key is. That sits under the profile's session-key prefix,
///   so the profile's lease fence ([`crate::profiles`]) guards a module's
///   writes exactly as it guards the profile's transcripts. Too long to be a
///   scope is an error, never a hash, because a hash would fall outside the
///   fence.
/// * Otherwise: `module:<id>@<base>`, or a hash of both when that is not a
///   valid scope or the id could make the form ambiguous.
///
/// Either way two modules never share a scope, and nothing the module sends
/// takes part.
pub(crate) fn module_scope(
    profile: Option<&str>,
    base: &Scope,
    module_id: &str,
) -> Result<Scope, StorageError> {
    let plain = !module_id.is_empty()
        && module_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    if let Some(profile) = profile {
        if !plain {
            return Err(StorageError::invalid_input("module id is not a plain name"));
        }
        let prefix = crate::core::runtime::session_key_prefix(profile);
        return Scope::new(format!("{prefix}module:{module_id}"));
    }
    if plain {
        if let Ok(scope) = Scope::new(format!("module:{module_id}@{}", base.as_str())) {
            return Ok(scope);
        }
    }
    use sha2::Digest;
    let mut digest = sha2::Sha256::new();
    digest.update(module_id.as_bytes());
    digest.update([0]);
    digest.update(base.as_str().as_bytes());
    Scope::new(format!("module-sha256:{}", hex::encode(digest.finalize())))
}

/// A module bound to a scope: its grant and the server that answers it.
struct Binding {
    backend: Arc<dyn StorageBackend>,
    grant: StorageGrant,
    server: StorageServer,
}

pub(crate) struct StorageHost {
    directory: Arc<dyn CallerDirectory>,
    environment: Arc<dyn Environment>,
    audit: Arc<dyn AuditSink>,
    bindings: tokio::sync::Mutex<HashMap<(String, String), Arc<Binding>>>,
    relays: std::sync::Mutex<HashMap<BusName, endpoint::Relay>>,
}

impl StorageHost {
    pub(crate) fn new(
        directory: Arc<dyn CallerDirectory>,
        environment: Arc<dyn Environment>,
        audit: Arc<dyn AuditSink>,
    ) -> Self {
        Self {
            directory,
            environment,
            audit,
            bindings: tokio::sync::Mutex::new(HashMap::new()),
            relays: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Resolves who is calling and decides the call: caller, scope, binding and
    /// grant. Everything a forwarded call needs, or the refusal.
    async fn admit(
        &self,
        object: Object,
        selector: &str,
        request: &Value,
        sender: Option<&BusName>,
    ) -> Result<Admitted, HostError> {
        let Some(sender) = sender else {
            tracing::warn!(
                target: "openhuman::storage_host",
                object = object.as_str(),
                op = selector,
                "[storage_host] refused a call with no authenticated sender"
            );
            return Err(HostError::Denied("unauthenticated-caller"));
        };
        let Some(module) = self.directory.module_for(sender).await else {
            tracing::warn!(
                target: "openhuman::storage_host",
                object = object.as_str(),
                op = selector,
                "[storage_host] refused a call from a peer that is not a registry module"
            );
            return Err(HostError::Denied("unknown-caller"));
        };
        let scope = self.environment.module_scope(&module);
        let call = AuditedCall {
            audit: Arc::clone(&self.audit),
            module,
            scope: scope
                .as_ref()
                .map_or_else(|_| "-".to_string(), |scope| scope.as_str().to_string()),
            object,
            op: selector.to_string(),
        };
        let scope = match scope {
            Ok(scope) => scope,
            Err(_) => {
                call.record("", "no-scope");
                return Err(HostError::Denied("no-scope"));
            }
        };
        let binding = match self.bind(&call.module, scope.clone()).await {
            Ok(binding) => binding,
            Err(error) => {
                call.record("", error.label());
                return Err(error.into_host());
            }
        };
        // Before the call reaches the server: a watch it opens must find the
        // caller's relay already listening.
        self.feed_relay(sender, &binding);
        let plan = match authorize(&binding.grant, object, selector, request) {
            Ok(plan) => plan,
            Err(denied) => {
                call.record("", denied.label());
                return Err(HostError::Denied(denied.label()));
            }
        };
        Ok(Admitted {
            call,
            scope,
            binding,
            plan,
        })
    }

    /// Consults the write fence when the plan writes. `Err` is already audited.
    async fn fence(&self, admitted: &Admitted, target: &str, write: bool) -> Result<(), HostError> {
        if !write {
            return Ok(());
        }
        if let Err(refused) = self.environment.admit_write(&admitted.scope).await {
            let kind = if crate::storage::fence::fence_error(&refused).is_some() {
                StorageKind::Fenced
            } else {
                StorageKind::Backend
            };
            admitted.call.record(target, "fenced");
            return Err(failure(kind));
        }
        Ok(())
    }

    /// Decides and answers one call.
    ///
    /// `sender` is the authenticated sender the bus delivered, never anything
    /// from the request. `admission` is `Confidential` only for a call the bus
    /// delivered on a confidential route.
    pub(crate) async fn handle(
        &self,
        object: Object,
        selector: &str,
        request: Value,
        sender: Option<&BusName>,
        admission: Admission,
    ) -> Result<Value, HostError> {
        let admitted = self.admit(object, selector, &request, sender).await?;
        match &admitted.plan {
            Plan::Answer(reply) => {
                admitted.call.record("", "answered-by-host");
                Ok(reply.clone())
            }
            Plan::Forward {
                target,
                filter,
                write,
            } => {
                self.fence(&admitted, target, *write).await?;
                match admitted
                    .binding
                    .server
                    .handle(admission, object.as_str(), selector, request)
                    .await
                {
                    Ok(reply) => {
                        admitted.call.record(target, "ok");
                        Ok(filter_reply(&admitted.binding.grant, *filter, reply))
                    }
                    Err(failed) => {
                        admitted.call.record(target, "storage-error");
                        Err(HostError::Storage(failed))
                    }
                }
            }
        }
    }

    /// `Blobs.PutStream`: stores `body`, which the bus delivered from a stream
    /// the authenticated caller opened, as exactly `head.len` bytes.
    pub(crate) async fn put_blob_stream(
        &self,
        sender: Option<&BusName>,
        head: PutBlobStreamRequest,
        body: BlobReader,
    ) -> Result<Value, HostError> {
        let request =
            serde_json::to_value(&head).map_err(|_| HostError::Denied("malformed-request"))?;
        let admitted = self
            .admit(Object::Blobs, "PutStream", &request, sender)
            .await?;
        let Plan::Forward { target, write, .. } = &admitted.plan else {
            return Err(HostError::Denied("malformed-request"));
        };
        if head.len > MAX_STREAMED_BLOB_BYTES {
            admitted.call.record(target, "blob-too-large");
            return Err(HostError::Denied("blob-too-large"));
        }
        self.fence(&admitted, target, *write).await?;
        match admitted.binding.server.put_blob_stream(head, body).await {
            Ok(info) => {
                admitted.call.record(target, "ok");
                serde_json::to_value(info).map_err(|_| HostError::Denied("encode-failed"))
            }
            Err(failed) => {
                admitted.call.record(target, "storage-error");
                Err(HostError::Storage(failed))
            }
        }
    }

    /// `Blobs.GetStream`: the blob's metadata and body, or `None`.
    pub(crate) async fn get_blob_stream(
        &self,
        sender: Option<&BusName>,
        request: BlobKeyRequest,
    ) -> Result<Option<(BlobStreamHead, BlobReader)>, HostError> {
        let wire =
            serde_json::to_value(&request).map_err(|_| HostError::Denied("malformed-request"))?;
        let admitted = self
            .admit(Object::Blobs, "GetStream", &wire, sender)
            .await?;
        let Plan::Forward { target, .. } = &admitted.plan else {
            return Err(HostError::Denied("malformed-request"));
        };
        match admitted.binding.server.get_blob_stream(request).await {
            Ok(found) => {
                admitted.call.record(target, "ok");
                Ok(found)
            }
            Err(failed) => {
                admitted.call.record(target, "storage-error");
                Err(HostError::Storage(failed))
            }
        }
    }

    /// The module's binding for `scope`, built on first use and again when the
    /// installed backend changed. The host runs the declared schema here.
    async fn bind(&self, module: &str, scope: Scope) -> Result<Arc<Binding>, BindError> {
        let backend = self.environment.backend().ok_or(BindError::NoBackend)?;
        let key = (module.to_string(), scope.as_str().to_string());
        let mut bindings = self.bindings.lock().await;
        if let Some(existing) = bindings.get(&key) {
            if Arc::ptr_eq(&existing.backend, &backend) {
                return Ok(Arc::clone(existing));
            }
        }
        let grant = self.environment.grant(module).ok_or(BindError::NoGrant)?;
        grant.validate().map_err(|reason| {
            tracing::error!(
                target: "openhuman::storage_host",
                module,
                reason,
                "[storage_host] refused an invalid storage grant"
            );
            BindError::BadGrant
        })?;
        let storage = backend
            .for_scope(&scope)
            .map_err(|_| BindError::Backend(StorageKind::Backend))?;
        for spec in grant.specs() {
            storage
                .documents()
                .ensure_collection(spec)
                .await
                .map_err(|_| BindError::Backend(StorageKind::Backend))?;
        }
        let mut server = StorageServer::new(storage.clone());
        if !grant.secret_prefixes.is_empty() {
            // A key-provider failure is retryable, so refuse the bind rather
            // than cache a server that silently lacks secrets.
            let keys = self.environment.keys().map_err(|_| {
                tracing::warn!(
                    target: "openhuman::storage_host",
                    module,
                    "[storage_host] no master key; refusing to bind a grant with secrets"
                );
                BindError::Backend(StorageKind::Backend)
            })?;
            server = server.with_secrets(Arc::new(DocumentSecrets::new(&storage, keys)));
        }
        let binding = Arc::new(Binding {
            backend,
            grant,
            server,
        });
        bindings.insert(key, Arc::clone(&binding));
        Ok(binding)
    }
}

/// A call that passed identification, with what answering it needs.
struct Admitted {
    call: AuditedCall,
    scope: Scope,
    binding: Arc<Binding>,
    plan: Plan,
}

/// The audit line of one call, minus its target and outcome.
struct AuditedCall {
    audit: Arc<dyn AuditSink>,
    module: String,
    scope: String,
    object: Object,
    op: String,
}

impl AuditedCall {
    fn record(&self, target: &str, outcome: &'static str) {
        self.audit.record(&AuditRecord {
            module: self.module.clone(),
            scope: self.scope.clone(),
            object: self.object.as_str(),
            op: self.op.clone(),
            target: target.to_string(),
            outcome,
        });
    }
}

enum BindError {
    NoBackend,
    NoGrant,
    BadGrant,
    Backend(StorageKind),
}

impl BindError {
    fn label(&self) -> &'static str {
        match self {
            Self::NoBackend => "no-backend",
            Self::NoGrant => "no-grant-declared",
            Self::BadGrant => "invalid-grant",
            Self::Backend(_) => "backend-error",
        }
    }

    fn into_host(self) -> HostError {
        match self {
            Self::NoBackend => failure(StorageKind::Unavailable),
            Self::NoGrant => HostError::Denied("no-grant-declared"),
            Self::BadGrant => HostError::Denied("invalid-grant"),
            Self::Backend(kind) => failure(kind),
        }
    }
}

/// The scope the host serves `module_id` under right now: what a bus call from
/// that module would be bound to. The host's own one-time legacy imports use it
/// to write a module's records where the module will read them.
///
/// # Errors
///
/// When the acting scope cannot be resolved (SaaS mode with no profile) or the
/// module id is not a plain name under a profile.
pub(crate) fn current_module_scope(
    ctx: &crate::core::runtime::DispatchCtx,
    module_id: &str,
) -> Result<Scope, StorageError> {
    let saas = crate::core::runtime::mode::is_saas();
    // A module call arrives on the module bus, outside any turn, so with no
    // tenant the host's default scope applies (refused in SaaS mode).
    let tenant = ctx.tenant().unwrap_or_default();
    let base =
        crate::storage::scope_from(tenant.profile.as_deref(), tenant.agent.as_deref(), saas)?;
    module_scope(tenant.profile.as_deref(), &base, module_id)
}

/// Production wiring: the broker's name owners, the installed backend, the
/// host's scope rule and the compiled grants.
struct Live {
    connection: tinybus::Connection,
}

#[async_trait]
impl CallerDirectory for Live {
    async fn module_for(&self, sender: &BusName) -> Option<String> {
        for record in super::registry::ALL {
            if let Ok(Some(owner)) = self.connection.name_owner(record.bus_name).await {
                if &owner == sender {
                    return Some(record.id.to_string());
                }
            }
        }
        None
    }
}

#[async_trait]
impl Environment for Live {
    fn backend(&self) -> Option<Arc<dyn StorageBackend>> {
        crate::storage::installed()
    }

    fn module_scope(&self, module_id: &str) -> Result<Scope, StorageError> {
        // A module's bus call carries no context of its own: it acts as the calling task.
        current_module_scope(&crate::core::runtime::DispatchCtx::ambient(), module_id)
    }

    fn grant(&self, module_id: &str) -> Option<StorageGrant> {
        declared(module_id)
    }

    fn keys(&self) -> Result<Arc<dyn KeyProvider>, StorageError> {
        crate::storage::secrets::keys()
    }

    async fn admit_write(&self, scope: &Scope) -> Result<(), StorageError> {
        crate::storage::fence::registry()
            .guard(scope.as_str(), "module-write")
            .await
    }
}

/// The production host: the live directory, environment and audit log.
pub(super) fn live_host(connection: &tinybus::Connection) -> Arc<StorageHost> {
    let live = Arc::new(Live {
        connection: connection.clone(),
    });
    Arc::new(StorageHost::new(live.clone(), live, Arc::new(LogAudit)))
}

/// Serves the four objects and claims the interface name on `connection`.
pub(super) async fn serve_objects(
    connection: &tinybus::Connection,
    host: &Arc<StorageHost>,
    runtime: Option<tokio::runtime::Handle>,
) -> tinybus::Result<tinystoragedrivers_busproxy::ServedStorage> {
    tinystoragedrivers_busproxy::serve(
        connection,
        Arc::new(HostEndpoint::new(Arc::clone(host), runtime)),
    )
    .await
}

/// Serves storage to modules.
///
/// Secrets travel only on tinybus' confidential route, and the broker delivers
/// that route to an attested recipient alone: a loaded module whose artifact
/// was verified, or code linked into this executable. The host's own client
/// connection is neither, so serving from it would refuse every secret
/// `NotAttested`. The objects are therefore served by a *linked* module
/// ([`super::storage_host_linked`]). If that cannot be attached the objects
/// are served from the host connection as before, with Secrets unreachable,
/// and the failure is logged.
pub(super) async fn install(
    connection: &tinybus::Connection,
    modules: &tinybus::module::ModuleHost,
) -> tinybus::Result<()> {
    let host = live_host(connection);
    match super::storage_host_linked::attach(modules, connection, Arc::clone(&host)).await {
        Ok(()) => {
            tracing::debug!("[storage_host] serving storage from the attested linked module");
            Ok(())
        }
        Err(error) => {
            tracing::warn!(
                %error,
                "[storage_host] linked storage module unavailable; module secrets are refused"
            );
            serve_objects(connection, &host, None).await.map(|served| {
                // The objects stay registered for the life of the connection.
                drop(served);
            })
        }
    }
}

#[path = "storage_host_endpoint.rs"]
mod endpoint;

#[cfg(test)]
#[path = "storage_host_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "storage_host_ctx_tests.rs"]
mod ctx_tests;
