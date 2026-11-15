//! Carrying a library host's secret grant on the context of the store it
//! authorises (`security::approval::gate::host_authorised_secret_grant`).

use super::*;

type HostSecretGrant = crate::security::approval::gate::HostSecretGrant;

impl CoreContext {
    /// The host secret grant on the context scoped onto this task, a
    /// [`bare`](Self::bare) one included. Whether it holds here is the
    /// grant's own check.
    pub(crate) fn current_host_secret_grant() -> Option<Arc<HostSecretGrant>> {
        Self::scoped_for_turn().and_then(|ctx| ctx.host_secret_grant.clone())
    }

    /// Run `fut` under this task's context carrying `grant`, or under a bare
    /// context carrying only it where no context is scoped (a library runtime
    /// storing its credential before its core boots). Every other reader
    /// answers exactly as it does outside.
    pub(crate) async fn scope_with_host_secret_grant<F: Future>(
        grant: Arc<HostSecretGrant>,
        fut: F,
    ) -> F::Output {
        let base = Self::scoped_for_turn().unwrap_or_else(Self::bare);
        tracing::debug!(
            bare = base.bare,
            "[core-context] scoping a host secret grant onto the current context"
        );
        CURRENT_CONTEXT
            .scope(Arc::new(base.copy_with(None, None, Some(grant))), fut)
            .await
    }
}
