//! [`MedullaClient`] methods for the public feedback board.
//!
//! The board is the user-facing half of the backend's feedback surface: list,
//! read, vote, and comment, plus submission through the shared hub ingest
//! endpoint. Admin triage endpoints are deliberately not modelled here — they
//! are gated to operators and have no place in the TUI.

use super::types::{
    CommentFeedbackBody, FeedbackDetail, FeedbackPage, FeedbackQuery, FeedbackSubmission,
    FeedbackType, SubmitFeedbackBody, VoteFeedbackBody,
};
use crate::openhuman::medulla::client::{urlencode, MedullaClient, Result};

/// The source product this client submits feedback as. Drives which repository
/// the backend's enrichment pipeline files the resulting issue into.
const FEEDBACK_PRODUCT: &str = "medulla";

/// The `origin` recorded on submissions, distinguishing TUI reports from the
/// web board in backend analytics.
const FEEDBACK_ORIGIN: &str = "medulla-tui";

impl MedullaClient {
    // --- Feedback board (/feedback) --------------------------------------

    /// List the public feedback board (`GET /feedback`).
    ///
    /// Returns only publicly visible items (open/planned/completed); the
    /// backend filters pending and moderation-rejected items out server-side.
    /// Each item carries the caller's own `my_vote`.
    pub async fn list_feedback(&self, query: &FeedbackQuery) -> Result<FeedbackPage> {
        let mut params: Vec<(&str, String)> = vec![
            ("sort", query.sort.as_str().to_string()),
            ("page", query.page.max(1).to_string()),
            ("limit", query.limit.clamp(1, 100).to_string()),
        ];
        // A `kind` of `Other` is a forward-compat placeholder, not a filter the
        // backend understands, so it is never sent.
        match query.kind {
            Some(FeedbackType::Other) | None => {}
            Some(kind) => params.push(("type", kind.as_str().to_string())),
        }
        if let Some(status) = query.status {
            params.push(("status", status.label().to_string()));
        }
        let req = self
            .authed(self.http.get(self.url("/feedback")))
            .query(&params);
        self.send(req).await
    }

    /// Fetch one board item with its comments (`GET /feedback/{id}`).
    pub async fn get_feedback(&self, id: &str) -> Result<FeedbackDetail> {
        let req = self.authed(
            self.http
                .get(self.url(&format!("/feedback/{}", urlencode(id)))),
        );
        self.send(req).await
    }

    /// Vote on a board item (`POST /feedback/{id}/vote`).
    ///
    /// `value` is `1` to upvote, `-1` to downvote, or `0` to retract an existing
    /// vote. Returns the item with recomputed tallies. Values outside that set
    /// are rejected by the backend with a 400.
    pub async fn vote_feedback(&self, id: &str, value: i8) -> Result<super::types::FeedbackItem> {
        let req = self
            .authed(
                self.http
                    .post(self.url(&format!("/feedback/{}/vote", urlencode(id)))),
            )
            .json(&VoteFeedbackBody { value });
        self.send(req).await
    }

    /// Comment on a board item (`POST /feedback/{id}/comments`).
    ///
    /// The backend rejects an empty body and caps length at 4000 characters.
    pub async fn comment_feedback(
        &self,
        id: &str,
        body: &str,
    ) -> Result<super::types::FeedbackComment> {
        let req = self
            .authed(
                self.http
                    .post(self.url(&format!("/feedback/{}/comments", urlencode(id)))),
            )
            .json(&CommentFeedbackBody { body });
        self.send(req).await
    }

    /// Submit feedback through the shared hub (`POST /feedback/ingest`).
    ///
    /// Uses the ingest endpoint rather than `POST /feedback` so the item is
    /// tagged with the `medulla` source product and the backend routes any
    /// filed issue to the medulla repository. `POST /feedback` would hardcode
    /// the source as `backend` and misroute the issue.
    ///
    /// Submissions are LLM-moderated and rate-limited (10/day by default).
    /// A moderation rejection is **not** an error: it returns `Ok` with
    /// [`FeedbackSubmission::accepted`] set to false and a `reason`. A
    /// rate-limit breach *is* an error (HTTP 429).
    pub async fn submit_feedback(
        &self,
        kind: FeedbackType,
        title: &str,
        body: &str,
    ) -> Result<FeedbackSubmission> {
        let req = self
            .authed(self.http.post(self.url("/feedback/ingest")))
            .json(&SubmitFeedbackBody {
                kind: kind.as_str(),
                title,
                body,
                product: FEEDBACK_PRODUCT,
                origin: FEEDBACK_ORIGIN,
            });
        self.send(req).await
    }
}
