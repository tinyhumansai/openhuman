//! What a loadable module may touch through the storage bus.
//!
//! A module reaches storage only through `ai.tinyhumans.tinystorage.Storage`
//! (`storage_host`). Its authority is a [`StorageGrant`] declared by the host
//! build for that module id: the collections it owns (with their schema), the
//! stream, blob and secret-name prefixes it may use, and whether it may write.
//! A module with no declaration has no storage, and a request that names
//! anything outside its grant is refused before it reaches the backend.
//!
//! [`authorize`] works on the wire request itself, because that is the one
//! place every selector's target is visible. It never trusts a field to
//! *widen* access: an unknown selector, a missing or non-string target and a
//! malformed batch are all refusals.
//!
//! Schema is host-only. The host runs `ensure_collection` for the declared
//! [`CollectionSpec`]s when it binds a module ([`StorageGrant::specs`]); a
//! module's own `EnsureCollection` is answered without touching the backend
//! when it repeats the declared spec, and refused otherwise. `DropCollection`
//! is never allowed.

use std::collections::BTreeSet;

use serde_json::Value;
use tinystoragedrivers::CollectionSpec;

pub(crate) use tinystoragedrivers_client::Object;

/// Whether the contract defines `selector` for `object`.
pub(crate) fn is_selector(object: Object, selector: &str) -> bool {
    use tinystoragedrivers_bus as wire;
    match object {
        Object::Documents => wire::is_documents_selector(selector),
        Object::Streams => wire::is_streams_selector(selector),
        Object::Blobs => wire::is_blobs_selector(selector),
        Object::Secrets => wire::is_secrets_selector(selector),
    }
}

/// Why a call was refused. Carries no request content, so it is safe to log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Denied {
    /// The grant does not include this object at all.
    Object,
    /// The target (collection, stream, blob key, secret name) is not granted.
    Target,
    /// The grant is read-only and the selector writes.
    ReadOnly,
    /// Schema and collection lifecycle belong to the host.
    Schema,
    /// The selector is not part of the contract, or carries no usable target.
    Malformed,
}

impl Denied {
    /// A stable label for logs.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Object => "object-not-granted",
            Self::Target => "target-not-granted",
            Self::ReadOnly => "read-only-grant",
            Self::Schema => "schema-is-host-only",
            Self::Malformed => "malformed-request",
        }
    }
}

/// How the reply of an allowed call must be trimmed to the grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReplyFilter {
    /// Pass the reply through.
    None,
    /// The reply is a list of collection names: keep the granted ones.
    GrantedCollections,
}

/// What to do with an allowed call.
#[derive(Debug, PartialEq)]
pub(crate) enum Plan {
    /// Forward the request to the bound server.
    Forward {
        /// What the call touches, for the audit line (a collection, a stream
        /// name or a granted prefix; never a payload).
        target: String,
        /// How to trim the reply.
        filter: ReplyFilter,
        /// Whether the call mutates storage, so the host fence is consulted
        /// before it goes to the backend.
        write: bool,
    },
    /// Answer without calling the backend (the host already ran it).
    Answer(Value),
}

/// One module's storage authority.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StorageGrant {
    /// Collections the module may use, each with the schema the host ensures.
    pub collections: Vec<CollectionSpec>,
    /// Stream-name prefixes the module may use.
    pub stream_prefixes: Vec<String>,
    /// Blob-key prefixes the module may use.
    pub blob_prefixes: Vec<String>,
    /// Secret-name prefixes the module may use (confidential callers only).
    pub secret_prefixes: Vec<String>,
    /// Whether the module may write; `false` makes every grant read-only.
    pub writable: bool,
}

/// Collection names no grant may hold: the encrypted secret documents and the
/// storage layer's own `_`-prefixed records.
const RESERVED_COLLECTIONS: [&str; 1] = [tinystoragedrivers::secrets::DEFAULT_COLLECTION];

impl StorageGrant {
    /// The schema the host ensures when it binds the module.
    pub fn specs(&self) -> &[CollectionSpec] {
        &self.collections
    }

    /// Refuses a grant that could reach beyond what a module may hold.
    ///
    /// # Errors
    ///
    /// A reserved or empty collection name, a duplicate collection, or an
    /// empty prefix (which would grant the module's whole scope).
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        for spec in &self.collections {
            if spec.name.is_empty()
                || spec.name.starts_with('_')
                || RESERVED_COLLECTIONS.contains(&spec.name.as_str())
            {
                return Err("a granted collection name is empty or reserved".into());
            }
            if !seen.insert(spec.name.as_str()) {
                return Err("a collection is granted twice".into());
            }
        }
        let prefixes = self
            .stream_prefixes
            .iter()
            .chain(&self.blob_prefixes)
            .chain(&self.secret_prefixes);
        if prefixes.into_iter().any(String::is_empty) {
            return Err("a granted prefix is empty".into());
        }
        Ok(())
    }

    fn has_object(&self, object: Object) -> bool {
        match object {
            Object::Documents => !self.collections.is_empty(),
            Object::Streams => !self.stream_prefixes.is_empty(),
            Object::Blobs => !self.blob_prefixes.is_empty(),
            Object::Secrets => !self.secret_prefixes.is_empty(),
        }
    }

    fn spec(&self, name: &str) -> Option<&CollectionSpec> {
        self.collections.iter().find(|spec| spec.name == name)
    }

    fn collection(&self, name: &str) -> Result<(), Denied> {
        self.spec(name).map(|_| ()).ok_or(Denied::Target)
    }

    fn prefixed<'a>(prefixes: &'a [String], name: &str) -> Result<&'a str, Denied> {
        prefixes
            .iter()
            .find(|prefix| name.starts_with(prefix.as_str()))
            .map(String::as_str)
            .ok_or(Denied::Target)
    }
}

/// The modules whose build declares storage, and what each may use.
///
/// The grant is generated from the module owner's `storage_schema()`
/// (`storage_grant_schemas`), so it cannot drift from what the module
/// declares; a module with no row has no storage at all. Rows: `tinychannels`,
/// `tinymcp`, `tinyconnectors`, `tinywallet`, `tinybox` and `tinyjuice`.
#[must_use]
pub fn declared(module_id: &str) -> Option<StorageGrant> {
    super::storage_grant_schemas::grant_for(module_id)
}

fn text<'a>(request: &'a Value, field: &str) -> Result<&'a str, Denied> {
    request
        .get(field)
        .and_then(Value::as_str)
        .ok_or(Denied::Malformed)
}

fn forward(target: impl Into<String>) -> Plan {
    Plan::Forward {
        target: target.into(),
        filter: ReplyFilter::None,
        write: false,
    }
}

/// Decides whether `grant` allows `object`'s `selector` with `request`, and
/// how to carry it out.
///
/// # Errors
///
/// A [`Denied`] reason when the grant does not cover the call.
pub(crate) fn authorize(
    grant: &StorageGrant,
    object: Object,
    selector: &str,
    request: &Value,
) -> Result<Plan, Denied> {
    if !is_selector(object, selector) {
        return Err(Denied::Malformed);
    }
    // `Capabilities` is how a module's storage client learns what the host
    // serves; the client probes every object when it connects and refuses to
    // bind when one is refused, so a grant without (say) blobs would leave the
    // whole module without storage. It reveals nothing of the data and every
    // other call on an ungranted object is still refused.
    if selector == "Capabilities" && !grant.has_object(object) {
        return match object {
            Object::Documents => Ok(Plan::Answer(serde_json::json!({ "capabilities": [] }))),
            Object::Blobs => Ok(Plan::Answer(serde_json::json!({
                "presigned_urls": false,
                "ttl": false,
                "max_key_len": 0,
            }))),
            Object::Streams | Object::Secrets => Err(Denied::Object),
        };
    }
    if !grant.has_object(object) {
        return Err(Denied::Object);
    }
    let write = is_write(object, selector);
    if write {
        writes(grant)?;
    }
    let plan = match object {
        Object::Documents => documents(grant, selector, request),
        Object::Streams => streams(grant, selector, request),
        Object::Blobs => blobs(grant, selector, request),
        Object::Secrets => secrets(grant, selector, request),
    }?;
    Ok(match plan {
        Plan::Forward { target, filter, .. } => Plan::Forward {
            target,
            filter,
            write,
        },
        answered @ Plan::Answer(_) => answered,
    })
}

/// Whether `selector` mutates storage.
fn is_write(object: Object, selector: &str) -> bool {
    matches!(
        (object, selector),
        (
            Object::Documents,
            "Put" | "Delete" | "DeleteWhere" | "Claim" | "AtomicBatch" | "AtomicBatchChecked"
        ) | (
            Object::Streams,
            "Append" | "AppendBatch" | "AppendBatchIfOffset" | "TruncateBefore" | "DeleteStream"
        ) | (Object::Blobs, "Put" | "PutStream" | "Delete" | "PresignPut")
            | (Object::Secrets, "Set" | "Delete")
    )
}

fn writes(grant: &StorageGrant) -> Result<(), Denied> {
    if grant.writable {
        Ok(())
    } else {
        Err(Denied::ReadOnly)
    }
}

fn documents(grant: &StorageGrant, selector: &str, request: &Value) -> Result<Plan, Denied> {
    match selector {
        "Get" | "Query" | "Count" | "Search" => {
            let collection = text(request, "collection")?;
            grant.collection(collection)?;
            Ok(forward(collection))
        }
        "Put" | "Delete" | "DeleteWhere" | "Claim" => {
            let collection = text(request, "collection")?;
            grant.collection(collection)?;
            Ok(forward(collection))
        }
        "AtomicBatch" | "AtomicBatchChecked" => batch(grant, request),
        "EnsureCollection" | "ValidateCollectionSpec" => {
            // The host ran `ensure_collection` from the declared spec when it
            // bound this module. Repeating it is harmless; changing it is not
            // the module's call.
            let wanted: CollectionSpec = request
                .get("spec")
                .cloned()
                .and_then(|spec| serde_json::from_value(spec).ok())
                .ok_or(Denied::Malformed)?;
            match grant.spec(&wanted.name) {
                Some(declared) if *declared == wanted => Ok(Plan::Answer(Value::Null)),
                _ => Err(Denied::Schema),
            }
        }
        "DropCollection" => Err(Denied::Schema),
        "Collections" => {
            // Listing is filtered, not refused: a module sees its own names.
            if !request.is_object() {
                return Err(Denied::Malformed);
            }
            Ok(Plan::Forward {
                target: String::new(),
                filter: ReplyFilter::GrantedCollections,
                write: false,
            })
        }
        // A watch is bound to the module's own server, and its invalidations
        // are unicast to that module's peer (`emit_to`), so the grant only has
        // to cover the collection. Closing names a handle the module's own
        // binding issued; another module's id is `NotFound` there.
        "WatchOpen" => {
            let collection = text(request, "collection")?;
            grant.collection(collection)?;
            Ok(forward(collection))
        }
        "WatchClose" => Ok(forward("")),
        "Capabilities" => Ok(forward("")),
        _ => Err(Denied::Malformed),
    }
}

fn batch(grant: &StorageGrant, request: &Value) -> Result<Plan, Denied> {
    let mut first: Option<&str> = None;
    for list in ["checks", "ops"] {
        let Some(entries) = request.get(list) else {
            if list == "ops" {
                return Err(Denied::Malformed);
            }
            continue;
        };
        for entry in entries.as_array().ok_or(Denied::Malformed)? {
            let collection = text(entry, "collection")?;
            grant.collection(collection)?;
            first.get_or_insert(collection);
        }
    }
    Ok(forward(first.unwrap_or_default()))
}

fn streams(grant: &StorageGrant, selector: &str, request: &Value) -> Result<Plan, Denied> {
    match selector {
        "ReadWindow" | "Len" => {
            let name = text(request, "stream")?;
            StorageGrant::prefixed(&grant.stream_prefixes, name)?;
            Ok(forward(name))
        }
        "Append" | "AppendBatch" | "AppendBatchIfOffset" | "TruncateBefore" | "DeleteStream" => {
            let name = text(request, "stream")?;
            StorageGrant::prefixed(&grant.stream_prefixes, name)?;
            Ok(forward(name))
        }
        "Streams" => {
            let prefix = text(request, "prefix")?;
            let granted = StorageGrant::prefixed(&grant.stream_prefixes, prefix)?;
            Ok(forward(granted))
        }
        _ => Err(Denied::Malformed),
    }
}

fn blobs(grant: &StorageGrant, selector: &str, request: &Value) -> Result<Plan, Denied> {
    match selector {
        // The streamed transfers carry the request head as `request`; the body
        // travels on a peer-bound bus stream the endpoint opens or accepts
        // (`Connection::accept_stream_from` / `open_stream`).
        "Get" | "GetStream" | "GetRange" | "Head" | "PresignGet" => {
            let key = text(request, "key")?;
            let granted = StorageGrant::prefixed(&grant.blob_prefixes, key)?;
            Ok(forward(granted))
        }
        "Put" | "PutStream" | "Delete" | "PresignPut" => {
            let key = text(request, "key")?;
            let granted = StorageGrant::prefixed(&grant.blob_prefixes, key)?;
            Ok(forward(granted))
        }
        "List" => {
            let prefix = text(request, "prefix")?;
            let granted = StorageGrant::prefixed(&grant.blob_prefixes, prefix)?;
            Ok(forward(granted))
        }
        "Capabilities" => Ok(forward("")),
        _ => Err(Denied::Malformed),
    }
}

fn secrets(grant: &StorageGrant, selector: &str, request: &Value) -> Result<Plan, Denied> {
    match selector {
        "Get" => {
            let granted = StorageGrant::prefixed(&grant.secret_prefixes, text(request, "name")?)?;
            Ok(forward(granted))
        }
        "Set" | "Delete" => {
            let granted = StorageGrant::prefixed(&grant.secret_prefixes, text(request, "name")?)?;
            Ok(forward(granted))
        }
        "List" => {
            let granted = StorageGrant::prefixed(&grant.secret_prefixes, text(request, "prefix")?)?;
            Ok(forward(granted))
        }
        _ => Err(Denied::Malformed),
    }
}

/// Trims a `Collections` reply to the names the grant holds.
pub(crate) fn filter_reply(grant: &StorageGrant, filter: ReplyFilter, reply: Value) -> Value {
    match (filter, reply) {
        (ReplyFilter::GrantedCollections, Value::Array(names)) => Value::Array(
            names
                .into_iter()
                .filter(|name| name.as_str().is_some_and(|name| grant.spec(name).is_some()))
                .collect(),
        ),
        (_, reply) => reply,
    }
}

#[cfg(test)]
#[path = "storage_grants_tests.rs"]
mod tests;
