//! Scope-bound storage handles guarded by a profile lease.
//!
//! When leases live in the storage backend, the driver evaluates their
//! persisted predicate atomically with each mutation. File leases have no
//! driver-visible record; their handles use the local fence checks, and the
//! OS lock cannot be lost while its process lives. `FencedBackend` composes a
//! driver predicate with any caller-supplied condition, so an importer claim
//! or other host condition cannot replace the profile's authority. A captured
//! backend keeps the exact lease epoch it was created for; it never resolves
//! a later registry entry for that profile.

use std::sync::Arc;

use tinystoragedrivers::{Scope, Storage, StorageBackend, StorageError};
use tinystoragedrivers_types::WriteCondition;

use super::fence::{FenceRegistry, LeaseFence};

type FencedWriteCondition = Option<(Arc<LeaseFence>, Option<WriteCondition>)>;

/// A backend whose scope handles carry the applicable host lease condition.
pub struct FencedBackend {
    inner: Arc<dyn StorageBackend>,
    registry: Option<Arc<FenceRegistry>>,
    captured: Option<Arc<LeaseFence>>,
}

impl std::fmt::Debug for FencedBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FencedBackend")
            .field("inner", &self.inner)
            .field("captured", &self.captured)
            .finish_non_exhaustive()
    }
}

impl FencedBackend {
    /// Create a backend that resolves the current registered fence for each
    /// newly opened scope.
    pub fn new(inner: Arc<dyn StorageBackend>, fences: Arc<FenceRegistry>) -> Self {
        Self {
            inner,
            registry: Some(fences),
            captured: None,
        }
    }

    /// Create a backend permanently tied to `fence`'s lease epoch.
    ///
    /// This is for work (such as maintenance) that must retain the authority
    /// admitted when a particular profile was captured. It fails closed for
    /// scopes outside the captured fence, and does not consult or follow a
    /// replacement fence in the registry.
    pub fn for_fence(
        inner: Arc<dyn StorageBackend>,
        clock: Arc<FenceRegistry>,
        fence: Arc<LeaseFence>,
    ) -> Self {
        Self {
            inner,
            // The registry supplies this host's clock only. Captured backends
            // never look up a replacement fence in it.
            registry: Some(clock),
            captured: Some(fence),
        }
    }

    fn condition_for(&self, scope: &Scope) -> Result<FencedWriteCondition, StorageError> {
        let fence = if let Some(captured) = &self.captured {
            if !captured.covers(scope.as_str()) {
                return Err(StorageError::backend(
                    "captured profile storage backend cannot open an out-of-fence scope",
                ));
            }
            Some(Arc::clone(captured))
        } else {
            self.registry
                .as_ref()
                .and_then(|registry| registry.lookup(scope.as_str()))
        };

        let Some(fence) = fence else {
            return Ok(None);
        };
        fence
            .check_local(
                self.registry
                    .as_ref()
                    .map_or_else(super::fence::system_now_ms, |registry| registry.now_ms()),
            )
            .map_err(|error| error.into_storage())?;
        Ok(Some((fence.clone(), fence.write_condition())))
    }

    /// The handle `scope`'s collections are declared through: the inner
    /// backend's, without the lease's driver condition, which MongoDB will
    /// not declare under (#8422). The fence still checks its lease locally
    /// before each declaration.
    fn declarations(
        &self,
        scope: &Scope,
    ) -> Result<Arc<dyn tinystoragedrivers_api::DocumentStore>, StorageError> {
        Ok(Arc::clone(self.inner.for_scope(scope)?.documents()))
    }
}

/// Wrap the process backend with its shared fence registry.
pub fn wrap(inner: Arc<dyn StorageBackend>) -> Arc<dyn StorageBackend> {
    Arc::new(FencedBackend::new(inner, super::fence::registry()))
}

impl StorageBackend for FencedBackend {
    fn capabilities(&self) -> tinystoragedrivers_types::Capabilities {
        self.inner.capabilities()
    }

    fn for_scope(&self, scope: &Scope) -> Result<Storage, StorageError> {
        match self.condition_for(scope)? {
            Some((fence, condition)) => {
                let (storage, declarations) = match condition {
                    Some(condition) => (
                        self.inner.for_scope_conditioned(scope, &condition)?,
                        self.declarations(scope)?,
                    ),
                    None => {
                        let storage = self.inner.for_scope(scope)?;
                        let declarations = Arc::clone(storage.documents());
                        (storage, declarations)
                    }
                };
                Ok(super::fenced_ports::wrap(
                    storage,
                    declarations,
                    fence,
                    Arc::clone(self.registry.as_ref().expect("fenced backend has a clock")),
                ))
            }
            None => self.inner.for_scope(scope),
        }
    }

    fn for_scope_conditioned(
        &self,
        scope: &Scope,
        condition: &WriteCondition,
    ) -> Result<Storage, StorageError> {
        match self.condition_for(scope)? {
            Some((fence, profile_condition)) => {
                let storage = match profile_condition {
                    Some(profile_condition) => self
                        .inner
                        .for_scope_conditioned(scope, &profile_condition.and(condition.clone())?)?,
                    None => self.inner.for_scope_conditioned(scope, condition)?,
                };
                Ok(super::fenced_ports::wrap(
                    storage,
                    self.declarations(scope)?,
                    fence,
                    Arc::clone(self.registry.as_ref().expect("fenced backend has a clock")),
                ))
            }
            None => self.inner.for_scope_conditioned(scope, condition),
        }
    }
}

#[cfg(test)]
#[path = "fenced_backend_tests.rs"]
mod tests;
