// Included into `gate.rs` beside `gate_state.rs`.

impl ApprovalGate {
    /// Dismiss a pending orphan: a request an earlier launch left undecided,
    /// which [`Self::list_pending`] lists for the default tenant. Nothing in
    /// this process can run the call it was for, so a denial is the only
    /// decision it takes; approving one is refused and it stays listed.
    ///
    /// `Ok(None)` when `request_id` is not a pending orphan, which `decide`
    /// reports as its miss.
    fn decide_orphan(
        &self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> anyhow::Result<Option<PendingApproval>> {
        let listed = match self.run_engine(|engine| async move { engine.orphans().await })? {
            Ok(listed) => listed,
            Err(error) => {
                tracing::warn!(
                    request_id = %request_id,
                    error = %error,
                    "[approval::gate] pending orphans could not be read for a decision"
                );
                return Ok(None);
            }
        };
        let Some(orphan) = listed.into_iter().find(|row| row.request_id == request_id) else {
            return Ok(None);
        };
        if decision != ApprovalDecision::Deny {
            tracing::debug!(
                request_id = %request_id,
                decision = decision.as_str(),
                "[approval::gate] refused approving a request an earlier launch left"
            );
            anyhow::bail!(
                "this request was left by an earlier launch; it can only be denied, \
                 because nothing in this launch can run it"
            );
        }
        let id = request_id.to_owned();
        let label = decision.as_str();
        match self.run_engine(|engine| async move { engine.dismiss_orphan(&id, label).await })? {
            Ok(()) => {}
            Err(EngineError::NotPending) => return Ok(None),
            Err(error) => {
                anyhow::bail!("the approval engine could not apply the decision: {error}")
            }
        }
        tracing::debug!(request_id = %request_id, "[approval::gate] dismissed a pending orphan");
        BUS.publish(DomainEvent::ApprovalDecided {
            request_id: request_id.to_string(),
            tool_name: orphan.tool_name.clone(),
            decision: decision.as_str().to_string(),
            thread_id: None,
            client_id: None,
            tool_call_id: None,
            resolution: None,
            agent_id: None,
        });
        Ok(Some(orphan))
    }
}
