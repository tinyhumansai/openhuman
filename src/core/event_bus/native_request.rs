//! Native, in-process typed request/response surface for the event bus.
//!
//! Unlike the broadcast (`publish_global` / `subscribe_global`) path which
//! fans events out to every subscriber, this is a **one-to-one request/response**
//! dispatcher keyed by a method string. Unlike a JSON-RPC registry, payloads
//! are **Rust types** — no serialization, no schema validation, no JSON. Trait
//! objects (`Arc<dyn Provider>`), streaming channels (`mpsc::Sender<T>`),
//! oneshot senders, and anything else `Send + 'static` all pass through
//! unchanged.
//!
//! Use this when one domain needs to call into another in-process and the
//! payload has a non-serializable shape (hot-path data, trait objects,
//! channels). For **fire-and-forget notification**, use the broadcast
//! surface instead.
//!
//! # Sync vs async
//!
//! * [`NativeRegistry::register`] / [`register_native_global`] are **sync** —
//!   registration is a trivial `HashMap::insert` guarded by a non-async
//!   `std::sync::RwLock`, so startup code in `Once::call_once` blocks or
//!   plain `fn main` can register handlers without an async runtime.
//! * [`NativeRegistry::request`] / [`request_native_global`] are **async** —
//!   they look up the handler under the read lock, clone its `Arc`, drop the
//!   lock, then `.await` the handler future. The lock is never held across
//!   an await point, so slow handlers never block other dispatches.
//!
//! # Usage
//!
//! ```ignore
//! // In a domain's bus.rs, called once at startup (sync):
//! register_native_global::<AgentTurnRequest, AgentTurnResponse, _, _>(
//!     "agent.run_turn",
//!     |req| async move {
//!         let text = run_tool_call_loop(/* ... */).await
//!             .map_err(|e| e.to_string())?;
//!         Ok(AgentTurnResponse { text })
//!     },
//! );
//!
//! // In a caller (async):
//! let resp: AgentTurnResponse = request_native_global(
//!     "agent.run_turn",
//!     AgentTurnRequest { /* owned + Arc fields */ },
//! ).await?;
//! ```
//!
//! # Testing
//!
//! Tests can override a handler by calling `register_native_global` again
//! for the same method — the most recent registration wins. For full
//! isolation, construct a fresh [`NativeRegistry`] directly and use
//! its `register` / `request` methods.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, OnceLock, RwLock};

use futures::future::BoxFuture;

/// Errors raised by the native (in-process, Rust-typed) request API.
#[derive(Debug, Clone)]
pub enum NativeRequestError {
    /// The global registry has not been initialized yet.
    NotInitialized,
    /// No handler registered for the given method name.
    UnregisteredHandler { method: String },
    /// Caller and registered handler disagree on request or response type.
    TypeMismatch {
        method: String,
        expected: &'static str,
        actual: &'static str,
    },
    /// The handler returned an error.
    HandlerFailed { method: String, message: String },
}

impl std::fmt::Display for NativeRequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInitialized => write!(f, "native request registry not initialized"),
            Self::UnregisteredHandler { method } => {
                write!(f, "no native handler registered for method '{method}'")
            }
            Self::TypeMismatch {
                method,
                expected,
                actual,
            } => write!(
                f,
                "native handler type mismatch for '{method}': expected {expected}, got {actual}"
            ),
            Self::HandlerFailed { method, message } => {
                write!(f, "native handler '{method}' failed: {message}")
            }
        }
    }
}

impl std::error::Error for NativeRequestError {}

// ── Internal type-erased storage ────────────────────────────────────────

type BoxedAny = Box<dyn Any + Send>;
type HandlerFuture = BoxFuture<'static, Result<BoxedAny, String>>;
type BoxedHandler = Arc<dyn Fn(BoxedAny) -> HandlerFuture + Send + Sync>;

struct HandlerEntry {
    handler: BoxedHandler,
    req_type: TypeId,
    resp_type: TypeId,
    req_name: &'static str,
    resp_name: &'static str,
}

// ── Registry ────────────────────────────────────────────────────────────

/// Registry of native, in-process typed request handlers.
///
/// Handlers are keyed by a method name (e.g., `"agent.run_turn"`) and store the
/// expected request and response types. This enables safe, typed communication
/// between different modules without the overhead of serialization.
///
/// The registry is thread-safe, using a `RwLock` to allow concurrent lookups
/// while guarding registrations.
#[derive(Clone, Default)]
pub struct NativeRegistry {
    /// Internal map of method names to their handler entries.
    handlers: Arc<RwLock<HashMap<String, HandlerEntry>>>,
}

impl std::fmt::Debug for NativeRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Non-blocking read attempt to avoid deadlocks during debugging.
        match self.handlers.try_read() {
            Ok(guard) => f
                .debug_struct("NativeRegistry")
                .field("methods", &guard.keys().collect::<Vec<_>>())
                .finish(),
            Err(_) => f
                .debug_struct("NativeRegistry")
                .field("methods", &"<locked>")
                .finish(),
        }
    }
}

/// Recover from `RwLock` poison by taking the inner guard.
///
/// If a thread panics while holding the lock, the lock becomes "poisoned".
/// Since the registry only holds a simple `HashMap`, we can safely ignore
/// the poison and continue using the registry.
fn unpoison<T>(result: Result<T, std::sync::PoisonError<T>>) -> T {
    result.unwrap_or_else(|e| e.into_inner())
}

impl NativeRegistry {
    /// Creates a new, empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a handler for a specific method name.
    ///
    /// If a handler already exists for the method, it will be replaced.
    /// This is particularly useful in tests for overriding production handlers
    /// with mocks or stubs.
    ///
    /// # Type Parameters
    ///
    /// * `Req` - The request type. Must implement `Send + 'static`.
    /// * `Resp` - The response type. Must implement `Send + 'static`.
    /// * `F` - The handler function/closure.
    /// * `Fut` - The future returned by the handler.
    pub fn register<Req, Resp, F, Fut>(&self, method: &str, handler: F)
    where
        Req: Send + 'static,
        Resp: Send + 'static,
        F: Fn(Req) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Resp, String>> + Send + 'static,
    {
        // Wrap the typed handler in a type-erased closure.
        let handler_arc: BoxedHandler = Arc::new(move |boxed: BoxedAny| {
            // This downcast is infallible: the dispatch path verifies
            // TypeId equality before invoking the handler.
            let req = *boxed
                .downcast::<Req>()
                .expect("native_request: dispatch passed wrong request type despite TypeId check");
            let fut = handler(req);
            Box::pin(async move { fut.await.map(|resp| Box::new(resp) as BoxedAny) })
        });

        let entry = HandlerEntry {
            handler: handler_arc,
            req_type: TypeId::of::<Req>(),
            resp_type: TypeId::of::<Resp>(),
            req_name: std::any::type_name::<Req>(),
            resp_name: std::any::type_name::<Resp>(),
        };

        // Insert the handler under a write lock.
        let previous = unpoison(self.handlers.write()).insert(method.to_string(), entry);

        if previous.is_some() {
            tracing::debug!(
                method,
                req_type = std::any::type_name::<Req>(),
                resp_type = std::any::type_name::<Resp>(),
                "[native_request] replaced existing handler"
            );
        } else {
            tracing::debug!(
                method,
                req_type = std::any::type_name::<Req>(),
                resp_type = std::any::type_name::<Resp>(),
                "[native_request] registered handler"
            );
        }
    }

    /// Dispatch a typed request to a registered handler.
    ///
    /// This method performs runtime type checks to ensure the caller and handler
    /// agree on the request and response types.
    ///
    /// # Errors
    ///
    /// Returns a [`NativeRequestError`] if:
    /// - No handler is registered for the method.
    /// - There is a type mismatch for the request or response.
    /// - The handler returns an error.
    pub async fn request<Req, Resp>(
        &self,
        method: &str,
        req: Req,
    ) -> Result<Resp, NativeRequestError>
    where
        Req: Send + 'static,
        Resp: Send + 'static,
    {
        // Lookup the handler and clone its metadata under a read lock.
        // We drop the lock BEFORE awaiting the handler's future to avoid
        // blocking other threads.
        let (handler, expected_req, expected_resp, expected_req_name, expected_resp_name) = {
            let guard = unpoison(self.handlers.read());
            let entry =
                guard
                    .get(method)
                    .ok_or_else(|| NativeRequestError::UnregisteredHandler {
                        method: method.to_string(),
                    })?;
            (
                Arc::clone(&entry.handler),
                entry.req_type,
                entry.resp_type,
                entry.req_name,
                entry.resp_name,
            )
        };

        // Verify that the caller's request type matches the registered type.
        if TypeId::of::<Req>() != expected_req {
            return Err(NativeRequestError::TypeMismatch {
                method: method.to_string(),
                expected: expected_req_name,
                actual: std::any::type_name::<Req>(),
            });
        }
        // Verify that the caller's response type matches the registered type.
        if TypeId::of::<Resp>() != expected_resp {
            return Err(NativeRequestError::TypeMismatch {
                method: method.to_string(),
                expected: expected_resp_name,
                actual: std::any::type_name::<Resp>(),
            });
        }

        tracing::debug!(
            method,
            req_type = std::any::type_name::<Req>(),
            "[native_request] dispatching"
        );

        let boxed_req: BoxedAny = Box::new(req);
        // Invoke the handler and await its completion.
        match handler(boxed_req).await {
            Ok(boxed_resp) => {
                // Infallible: the TypeId check above guarantees the correct type.
                let resp = *boxed_resp.downcast::<Resp>().expect(
                    "native_request: handler returned wrong response type despite TypeId check",
                );
                tracing::debug!(method, "[native_request] dispatch completed");
                Ok(resp)
            }
            Err(message) => {
                tracing::debug!(method, %message, "[native_request] handler returned error");
                Err(NativeRequestError::HandlerFailed {
                    method: method.to_string(),
                    message,
                })
            }
        }
    }

    /// Returns `true` if a handler is registered for `method`.
    pub fn is_registered(&self, method: &str) -> bool {
        unpoison(self.handlers.read()).contains_key(method)
    }

    /// Returns the number of registered handlers (useful for tests and
    /// startup smoke checks).
    pub fn len(&self) -> usize {
        unpoison(self.handlers.read()).len()
    }

    /// Returns `true` if no handlers are registered.
    pub fn is_empty(&self) -> bool {
        unpoison(self.handlers.read()).is_empty()
    }

    /// Remove all registered handlers. Intended for tests only.
    pub fn clear(&self) {
        unpoison(self.handlers.write()).clear();
    }
}

// ── Global singleton ────────────────────────────────────────────────────

static GLOBAL_REGISTRY: OnceLock<NativeRegistry> = OnceLock::new();

/// Initialize the global native request registry. Idempotent — safe to
/// call multiple times. Returns the singleton.
pub fn init_native_registry() -> &'static NativeRegistry {
    GLOBAL_REGISTRY.get_or_init(|| {
        tracing::debug!("[native_request] initializing global registry");
        NativeRegistry::new()
    })
}

/// Get the global native request registry, or `None` if not initialized.
pub fn native_registry() -> Option<&'static NativeRegistry> {
    GLOBAL_REGISTRY.get()
}

/// Register a handler on the global native registry. Auto-initializes
/// the registry on first call — this is the canonical entry point used
/// by domain `bus.rs` files at startup.
///
/// Synchronous: can be called from `fn main`, `Once::call_once`, or any
/// non-async context.
pub fn register_native_global<Req, Resp, F, Fut>(method: &str, handler: F)
where
    Req: Send + 'static,
    Resp: Send + 'static,
    F: Fn(Req) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Resp, String>> + Send + 'static,
{
    init_native_registry().register(method, handler);
}

/// Dispatch a typed request on the global native registry.
///
/// Returns [`NativeRequestError::NotInitialized`] if no handler has been
/// registered yet (which implicitly initializes the registry) — callers
/// hitting this usually have a startup ordering bug.
pub async fn request_native_global<Req, Resp>(
    method: &str,
    req: Req,
) -> Result<Resp, NativeRequestError>
where
    Req: Send + 'static,
    Resp: Send + 'static,
{
    let registry = native_registry().ok_or(NativeRequestError::NotInitialized)?;
    registry.request(method, req).await
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "native_request_tests.rs"]
mod tests;
