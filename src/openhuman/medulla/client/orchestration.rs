//! One-shot orchestration runs (`/orchestration/v1`).
//!
//! Split from the client transport core; the shared request helpers
//! (`url`, `authed`, `send`) live in [`super`].

use super::*;

impl MedullaClient {
    // --- Orchestration ---------------------------------------------------

    /// One-shot orchestration run (`POST /orchestration/v1/run`).
    ///
    /// Without tools the backend returns a final reply; with tools it returns
    /// the first [`LoopEvent`].
    pub async fn run(&self, input: &str, options: RunOptions) -> Result<RunResult> {
        let req = self
            .authed(self.http.post(self.url("/orchestration/v1/run")))
            .json(&RunBody {
                input,
                options: &options,
            });
        let value: Value = self.send(req).await?;
        parse_run_result(value)
    }

    /// Continue a tool-loop run (`POST /orchestration/v1/run/continue`).
    ///
    /// Pass an empty `tool_results` to poll a pending run.
    pub async fn continue_run(
        &self,
        cycle_id: &str,
        tool_results: Vec<ToolResult>,
    ) -> Result<LoopEvent> {
        let req = self
            .authed(self.http.post(self.url("/orchestration/v1/run/continue")))
            .json(&ContinueRunBody {
                cycle_id,
                tool_results,
            });
        self.send(req).await
    }
}

/// Decide whether a run response is a tool-less reply or a tool-loop event.
pub(crate) fn parse_run_result(value: Value) -> Result<RunResult> {
    if value.get("stop").is_some() {
        let ev: LoopEvent =
            serde_json::from_value(value).map_err(|e| ClientError::Decode(e.to_string()))?;
        Ok(RunResult::Loop(ev))
    } else {
        let reply: RunReply =
            serde_json::from_value(value).map_err(|e| ClientError::Decode(e.to_string()))?;
        Ok(RunResult::Reply(reply))
    }
}
