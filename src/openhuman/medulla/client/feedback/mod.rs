//! The public feedback board client surface (`/feedback`).
//!
//! [`types`] holds the wire shapes; [`api`] adds the [`MedullaClient`] methods
//! that speak them.
//!
//! [`MedullaClient`]: crate::openhuman::medulla::client::MedullaClient

mod api;
pub mod types;

pub use types::{
    FeedbackComment, FeedbackDetail, FeedbackGithub, FeedbackItem, FeedbackPage, FeedbackQuery,
    FeedbackSort, FeedbackStatus, FeedbackSubmission, FeedbackType,
};

#[cfg(test)]
#[path = "feedback_tests.rs"]
mod tests;
