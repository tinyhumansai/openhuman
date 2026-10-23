//! Spawning tasks without dropping who they work for.
//!
//! A tokio task starts with no ambient [`CoreContext`] (it is a task-local), so
//! a bare `tokio::spawn` falls back to the process default context. In a
//! single-user process that is the right user by accident; in SaaS it is the
//! operator, so a dropped scope writes one user's work somewhere it does not
//! belong, or fails.
//!
//! The context travels as a value instead: a spawner holds a [`DispatchCtx`]
//! and spawns through [`DispatchCtx::spawn`] / [`DispatchCtx::spawn_blocking`],
//! which hand it to the task. A task whose body acts on no tenant state
//! (a signal relay, a forwarder, a process-wide sweeper) is a plain
//! `tokio::spawn`.

use std::future::Future;

use tokio::task::JoinHandle;

use super::{CoreContext, DispatchCtx};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockOnError {
    Runtime,
    Panicked,
}

impl DispatchCtx {
    /// Run a future on a dedicated current-thread runtime while preserving
    /// this dispatch context. This is for synchronous APIs that must bridge to
    /// async module calls and may be called outside a Tokio runtime.
    pub(crate) fn block_on<F, T>(&self, future: F) -> Result<T, BlockOnError>
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let dispatch = self.clone();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| BlockOnError::Runtime)?;
            Ok(runtime.block_on(dispatch.enter(future)))
        })
        .join()
        .map_err(|_| BlockOnError::Panicked)?
    }

    /// `fut`, wrapped to run under this context wherever it is polled. An
    /// unbound context runs it bare, which in SaaS fails closed.
    pub fn enter<F>(&self, fut: F) -> impl Future<Output = F::Output> + Send
    where
        F: Future + Send,
        F::Output: Send,
    {
        let ctx = self.core().cloned();
        #[cfg(test)]
        let prompt_scanner = crate::security::prompt_injection::current_scoped();
        // Box before the task-local wrapper. Its branching async state otherwise
        // multiplies a large task's inline size before Tokio can box it,
        // overflowing the spawning thread (including the CLI's main stack).
        let fut = Box::pin(fut);
        let entered = async move {
            match ctx {
                Some(ctx) => CoreContext::scope(ctx, fut).await,
                None => fut.await,
            }
        };
        #[cfg(test)]
        {
            async move {
                if let Some(scanner) = prompt_scanner {
                    return crate::security::prompt_injection::with_scanner(scanner, entered).await;
                }
                entered.await
            }
        }
        #[cfg(not(test))]
        {
            entered
        }
    }

    /// Like `tokio::spawn`, but the task acts under this context.
    pub fn spawn<F>(&self, fut: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        tokio::spawn(self.enter(fut))
    }

    /// Like `tokio::task::spawn_blocking`, but the closure runs under this
    /// context.
    pub fn spawn_blocking<F, R>(&self, f: F) -> JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let ctx = self.core().cloned();
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || match ctx {
            Some(ctx) => handle.block_on(CoreContext::scope(ctx, async move { f() })),
            None => f(),
        })
    }
}

/// The context a carried future inherits. In SaaS only the caller's own task
/// scope is carried: an unscoped caller's task gets no context (and so fails
/// closed) rather than the operator's default one.
fn captured() -> Option<std::sync::Arc<CoreContext>> {
    if super::is_saas() {
        CoreContext::scoped()
    } else {
        CoreContext::current()
    }
}

/// What [`CoreContext::propagate`] wraps a future in: the caller's context
/// captured now and re-entered wherever the future is polled.
///
/// Single-user processes carry [`CoreContext::current`]. In SaaS only the
/// caller's own task scope is carried (an unscoped caller's future runs with
/// no context and so fails closed), exactly as [`scoped`] does — but without
/// requiring the future to be `Send`.
pub(crate) fn carry<F: Future>(fut: F) -> impl Future<Output = F::Output> {
    let ctx = captured();
    #[cfg(test)]
    let prompt_scanner = crate::security::prompt_injection::current_scoped();
    async move {
        let carried = async move {
            match ctx {
                Some(ctx) => CoreContext::scope(ctx, fut).await,
                None => fut.await,
            }
        };
        #[cfg(test)]
        if let Some(scanner) = prompt_scanner {
            return crate::security::prompt_injection::with_scanner(scanner, carried).await;
        }
        carried.await
    }
}

#[cfg(test)]
#[path = "spawn_tests.rs"]
mod tests;
