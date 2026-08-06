//! Wire types for the backend's public feedback board (`/feedback`).
//!
//! These mirror the backend's `SerializedFeedback` / `SerializedComment` shapes.
//! Enums that cross the wire carry an `Other` fallback so a backend that grows a
//! new type or status does not break older clients.

use serde::{Deserialize, Serialize};

/// What a board item is: a feature request or a bug report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackType {
    /// A requested capability.
    Feature,
    /// A defect report.
    Bug,
    /// Any type not yet modelled by this client.
    #[serde(other)]
    Other,
}

impl FeedbackType {
    /// The wire value the backend expects when submitting.
    pub fn as_str(self) -> &'static str {
        match self {
            FeedbackType::Feature => "feature",
            FeedbackType::Bug => "bug",
            FeedbackType::Other => "feature",
        }
    }

    /// A short label for list rows.
    pub fn label(self) -> &'static str {
        match self {
            FeedbackType::Feature => "feat",
            FeedbackType::Bug => "bug",
            FeedbackType::Other => "misc",
        }
    }
}

/// Where a board item sits in the triage lifecycle.
///
/// Only `Open`/`Planned`/`Completed` are ever public; the backend filters the
/// rest out of board responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackStatus {
    /// Accepted and awaiting triage.
    Open,
    /// Accepted onto the roadmap.
    Planned,
    /// Shipped.
    Completed,
    /// Any status not yet modelled by this client.
    #[serde(other)]
    Other,
}

impl FeedbackStatus {
    /// A short label for list rows.
    pub fn label(self) -> &'static str {
        match self {
            FeedbackStatus::Open => "open",
            FeedbackStatus::Planned => "planned",
            FeedbackStatus::Completed => "done",
            FeedbackStatus::Other => "?",
        }
    }
}

/// The GitHub issue a board item was filed as, once the enrichment pipeline has
/// filed it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackGithub {
    /// The issue number in the target repo.
    pub issue_number: Option<i64>,
    /// The issue's web URL.
    pub issue_url: Option<String>,
}

/// One item on the public feedback board.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackItem {
    /// The item's backend id, used for vote/comment calls.
    pub id: String,
    /// Feature request or bug report.
    #[serde(rename = "type")]
    pub kind: FeedbackType,
    /// The (moderation-sanitized) title.
    pub title: String,
    /// The (moderation-sanitized) body.
    pub body: String,
    /// Triage status.
    pub status: FeedbackStatus,
    /// The author's display name, when the backend could resolve one.
    pub created_by_name: Option<String>,
    /// Total upvotes.
    pub upvote_count: i64,
    /// Total downvotes.
    pub downvote_count: i64,
    /// `upvote_count - downvote_count`.
    pub score: i64,
    /// Number of comments on the item.
    pub comment_count: i64,
    /// The filed GitHub issue, once the pipeline has filed one.
    #[serde(default)]
    pub github: Option<FeedbackGithub>,
    /// The requesting user's own vote: `1`, `-1`, or `0` for none.
    #[serde(default)]
    pub my_vote: i8,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
}

/// One comment on a board item.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackComment {
    /// The comment's backend id.
    pub id: String,
    /// The commenter's display name, when resolvable.
    pub user_name: Option<String>,
    /// The comment text.
    pub body: String,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
}

/// A page of the board (`GET /feedback`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackPage {
    /// The items on this page.
    pub items: Vec<FeedbackItem>,
    /// Total items matching the filter, across all pages.
    pub total: i64,
}

/// A single item plus its comments (`GET /feedback/{id}`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackDetail {
    /// The item itself.
    pub feedback: FeedbackItem,
    /// Its comments, oldest first.
    pub comments: Vec<FeedbackComment>,
}

/// The result of submitting feedback (`POST /feedback/ingest`).
///
/// Submissions are LLM-moderated: a rejected submission still returns HTTP 200
/// with `accepted == false`, so callers **must** check this flag rather than
/// assuming success from the status code.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackSubmission {
    /// Whether moderation accepted the item onto the board.
    pub accepted: bool,
    /// The moderator's reason; shown to the user when `accepted` is false.
    #[serde(default)]
    pub reason: String,
    /// The created item, present only when `accepted`.
    #[serde(default)]
    pub feedback: Option<FeedbackItem>,
}

/// How the board is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackSort {
    /// Time-decayed score (the backend default).
    Hot,
    /// Raw score.
    Top,
    /// Newest first.
    New,
}

impl FeedbackSort {
    /// The wire value for the `sort` query parameter.
    pub fn as_str(self) -> &'static str {
        match self {
            FeedbackSort::Hot => "hot",
            FeedbackSort::Top => "top",
            FeedbackSort::New => "new",
        }
    }

    /// The next sort in the cycle, for a single-key toggle.
    pub fn next(self) -> Self {
        match self {
            FeedbackSort::Hot => FeedbackSort::Top,
            FeedbackSort::Top => FeedbackSort::New,
            FeedbackSort::New => FeedbackSort::Hot,
        }
    }
}

/// A board query: filters, ordering, and pagination.
#[derive(Debug, Clone)]
pub struct FeedbackQuery {
    /// Restrict to one item type; `None` lists both.
    pub kind: Option<FeedbackType>,
    /// Restrict to one status; `None` lists all public statuses.
    pub status: Option<FeedbackStatus>,
    /// Ordering.
    pub sort: FeedbackSort,
    /// 1-based page number.
    pub page: u32,
    /// Page size; the backend caps this at 100.
    pub limit: u32,
}

impl Default for FeedbackQuery {
    fn default() -> Self {
        Self {
            kind: None,
            status: None,
            sort: FeedbackSort::Hot,
            page: 1,
            limit: 50,
        }
    }
}

/// Request body for changing the caller's vote on a feedback item.
#[derive(serde::Serialize)]
pub(super) struct VoteFeedbackBody {
    pub(super) value: i8,
}

/// Request body for adding a comment to a feedback item.
#[derive(serde::Serialize)]
pub(super) struct CommentFeedbackBody<'a> {
    pub(super) body: &'a str,
}

/// Request body for submitting feedback through the shared hub.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SubmitFeedbackBody<'a> {
    #[serde(rename = "type")]
    pub(super) kind: &'a str,
    pub(super) title: &'a str,
    pub(super) body: &'a str,
    pub(super) product: &'a str,
    pub(super) origin: &'a str,
}
