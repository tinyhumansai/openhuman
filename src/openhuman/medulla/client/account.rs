//! Account-scoped reads: team token usage and the onboarding history reward.
//!
//! Split from the client transport core; the shared request helpers
//! (`url`, `authed`, `send`) live in [`super`].

use super::*;

impl MedullaClient {
    /// Account-level token usage for the active team/user
    /// (`GET /teams/me/usage`): cycle window, spend, per-model breakdown,
    /// remaining budget, plan. Returned as raw JSON — the shape is rendered
    /// defensively by the UI.
    pub async fn team_usage(&self) -> Result<Value> {
        let req = self.authed(self.http.get(self.url("/teams/me/usage")));
        self.send(req).await
    }

    // --- History rewards -------------------------------------------------

    /// Upload one redacted session transcript toward the onboarding reward
    /// (`POST /agent-integrations/history-rewards/uploads`).
    ///
    /// The caller is responsible for redacting `content` first — see
    /// the host-side history-upload redactor. The backend encrypts what it receives before
    /// storing it, and returns the claim's running metrics.
    ///
    /// Errors with [`ClientError::Api`] when the reward was already claimed, the
    /// session cap is reached, or the transcript exceeds the size limit.
    pub async fn upload_history_session(
        &self,
        agent: &str,
        content: String,
    ) -> Result<HistoryUploadResult> {
        let part = reqwest::multipart::Part::text(content)
            .file_name("session.jsonl")
            .mime_str("application/x-ndjson")?;
        let form = reqwest::multipart::Form::new()
            .text("agent", agent.to_string())
            .part("file", part);

        let req = self.authed(
            self.http
                .post(self.url("/agent-integrations/history-rewards/uploads"))
                .multipart(form),
        );
        self.send(req).await
    }

    /// Score the uploaded history and grant the welcome credit
    /// (`POST /agent-integrations/history-rewards/claim`).
    ///
    /// Idempotent: a repeat call returns the same award with
    /// `already_claimed` set rather than granting again.
    pub async fn claim_history_reward(&self) -> Result<HistoryRewardClaim> {
        let req = self.authed(
            self.http
                .post(self.url("/agent-integrations/history-rewards/claim")),
        );
        self.send(req).await
    }

    /// Whether the caller has already earned the history reward
    /// (`GET /agent-integrations/history-rewards/status`).
    ///
    /// A user who has never uploaded gets a zeroed, unclaimed status rather
    /// than an error.
    pub async fn history_reward_status(&self) -> Result<HistoryRewardStatus> {
        let req = self.authed(
            self.http
                .get(self.url("/agent-integrations/history-rewards/status")),
        );
        self.send(req).await
    }
}
