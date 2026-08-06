//! Durable session lifecycle (`/medulla/v1/sessions`) and its SSE stream.
//!
//! Split from the client transport core; the shared request helpers
//! (`url`, `authed`, `send`) live in [`super`].

use super::*;
use futures::stream::Stream;

impl MedullaClient {
    // --- Sessions --------------------------------------------------------

    /// Create a durable session (`POST /medulla/v1/sessions`).
    pub async fn create_session(&self, title: Option<&str>) -> Result<SessionCreated> {
        self.create_session_with(title, &[]).await
    }

    /// Create a durable session, attaching authored workspace profiles to the mint
    /// (`POST /medulla/v1/sessions` with `workspaceProfiles`).
    ///
    /// Each profile is one workspace root's verbatim `MEDULLA.md`, collected via
    /// the host-side workspace-profile collector. An empty slice mints a plain
    /// session (the `workspaceProfiles` key is omitted). The backend validates the
    /// shape and rejects a malformed profile with a 400.
    pub async fn create_session_with(
        &self,
        title: Option<&str>,
        workspace_profiles: &[WorkspaceProfileInput],
    ) -> Result<SessionCreated> {
        let req = self
            .authed(self.http.post(self.url("/medulla/v1/sessions")))
            .json(&CreateSessionBody {
                title,
                workspace_profiles,
            });
        self.send(req).await
    }

    /// List sessions (`GET /medulla/v1/sessions`).
    pub async fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        let req = self.authed(self.http.get(self.url("/medulla/v1/sessions")));
        self.send(req).await
    }

    /// Fetch a session's state (`GET /medulla/v1/sessions/:id`).
    pub async fn get_session(&self, session_id: &str) -> Result<SessionDetail> {
        let req = self.authed(
            self.http
                .get(self.url(&format!("/medulla/v1/sessions/{session_id}"))),
        );
        self.send(req).await
    }

    /// Archive a session (`DELETE /medulla/v1/sessions/:id`).
    pub async fn archive_session(&self, session_id: &str) -> Result<SessionArchived> {
        let req = self.authed(
            self.http
                .delete(self.url(&format!("/medulla/v1/sessions/{session_id}"))),
        );
        self.send(req).await
    }

    /// Send a message (`POST /medulla/v1/sessions/:id/messages`).
    ///
    /// With `sync = false` the backend returns 202 `{cycleId, seq}`; with
    /// `sync = true` it blocks and returns `{cycleId, seq, reply}`.
    pub async fn send_message(
        &self,
        session_id: &str,
        body: &str,
        sync: bool,
    ) -> Result<SendResult> {
        let sync_flag = if sync { "1" } else { "0" };
        let req = self
            .authed(
                self.http
                    .post(self.url(&format!("/medulla/v1/sessions/{session_id}/messages")))
                    .query(&[("sync", sync_flag)]),
            )
            .json(&SendMessageBody { body });
        self.send(req).await
    }

    /// Replay messages after `after` (`GET .../messages?after=`).
    pub async fn list_messages(
        &self,
        session_id: &str,
        after: Option<i64>,
    ) -> Result<Vec<Message>> {
        let mut req = self
            .http
            .get(self.url(&format!("/medulla/v1/sessions/{session_id}/messages")));
        if let Some(after) = after {
            req = req.query(&[("after", after)]);
        }
        self.send(self.authed(req)).await
    }

    /// Replay events after `after` (`GET .../events?after=`).
    pub async fn list_events(
        &self,
        session_id: &str,
        after: Option<i64>,
    ) -> Result<Vec<EventEnvelope>> {
        let mut req = self
            .http
            .get(self.url(&format!("/medulla/v1/sessions/{session_id}/events")));
        if let Some(after) = after {
            req = req.query(&[("after", after)]);
        }
        self.send(self.authed(req)).await
    }

    /// Abort the running cycle (`POST /medulla/v1/sessions/:id/abort`).
    pub async fn abort(&self, session_id: &str) -> Result<AbortResult> {
        let req = self.authed(
            self.http
                .post(self.url(&format!("/medulla/v1/sessions/{session_id}/abort"))),
        );
        self.send(req).await
    }
    // --- SSE -------------------------------------------------------------

    /// Open a reconnecting SSE stream of events for a session
    /// (`GET /medulla/v1/sessions/:id/stream?token=<jwt>`).
    ///
    /// The returned stream auto-reconnects with `Last-Event-ID` and
    /// de-duplicates replayed frames by seq. Drop it to stop.
    pub fn stream_events(
        &self,
        session_id: &str,
        last_event_id: Option<u64>,
    ) -> impl Stream<Item = Result<EventEnvelope>> {
        let url = format!(
            "{}/medulla/v1/sessions/{}/stream?token={}",
            self.base_url,
            session_id,
            urlencode(&self.jwt),
        );
        sse::event_stream(self.http.clone(), url, last_event_id)
    }
}
