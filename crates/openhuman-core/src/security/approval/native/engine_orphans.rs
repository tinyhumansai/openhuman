//! Dismissing a pending orphan: a request an earlier launch left undecided,
//! which the legacy import carried into the module as a pending row no host
//! instance owns. Nothing in this process can run the call it was for, so the
//! module verifies only a denial for it (`Decide2`), and the host offers only
//! that.

use super::*;
use crate::core::runtime::Tenant;

impl NativeEngine {
    /// Deny the pending orphan `request_id` as a person's decision.
    /// `Err(NotPending)` when it is not a pending orphan of this workspace: a
    /// request of this process, one already decided or expired, or no row.
    pub(super) async fn dismiss(&self, request_id: &str, label: &str) -> Result<(), EngineError> {
        if host().request(request_id).is_some() {
            return Err(EngineError::NotPending);
        }
        let proxy = self.proxy().await?;
        // The import files every legacy row under the default tenant's
        // standard subject, so that is the only scope an orphan lives in.
        let scope = self.history_scope(&proxy, &Tenant::default()).await?;
        let row = scope
            .rows
            .row(request_id)
            .await
            .map_err(EngineError::Unavailable)?
            .map(|stored| stored.row)
            .filter(|row| {
                row.state == ApprovalLifecycle2::Pending
                    && row.legacy_history_json.is_some()
                    && row.expires_at_ms > now_ms()
            })
            .ok_or(EngineError::NotPending)?;
        if !host().begin_orphan_review(row) {
            return Err(unavailable(
                "the earlier request is already being dismissed",
            ));
        }
        let ticket = opaque();
        host().issue(
            ticket.clone(),
            Ticket::new(
                request_id.to_owned(),
                ApprovalChoice2::Deny,
                ACTOR.to_owned(),
            ),
        );
        let decided = async {
            let _guard = scope.stream.lock().await;
            let generation = Self::generation(&proxy).await?;
            let audit = Self::correlation(&scope, generation).await?;
            proxy
                .call_confidential::<ApprovalRow2>(
                    names::methods::DECIDE2,
                    (DecideRequest2 {
                        scope_id: scope.scope_id.clone(),
                        request_id: request_id.to_owned(),
                        decision_ticket: ticket.clone(),
                        audit,
                    },),
                )
                .await
                .map_err(|error| {
                    tracing::debug!(error = %error, "{LOG} the module refused to dismiss the orphan");
                    unavailable(format!("TinySecurity Decide2 failed: {error}"))
                })
        }
        .await;
        host().revoke(&ticket);
        host().end_orphan_review(request_id);
        match decided {
            Ok(row) if row.state == ApprovalLifecycle2::Denied => {
                if let Err(error) = scope.rows.put_label(request_id, label).await {
                    tracing::debug!(error = %error, "{LOG} the dismissal label was not kept");
                }
                tracing::debug!(request_id = %request_id, "{LOG} dismissed a pending orphan");
                Ok(())
            }
            Ok(_) => Err(EngineError::NotPending),
            Err(error) => match scope.rows.row(request_id).await {
                // A competing decision or the expiry got there first.
                Ok(Some(stored)) if stored.row.state != ApprovalLifecycle2::Pending => {
                    Err(EngineError::NotPending)
                }
                _ => Err(error),
            },
        }
    }
}
