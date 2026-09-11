//! LinkedIn profile enrichment via Gmail email mining + Apify scraping.
//!
//! Pipeline:
//!
//! 1. Search Gmail (via Composio) for emails from `linkedin.com`.
//! 2. Extract a `linkedin.com/in/<slug>` profile URL from the results.
//! 3. Scrape the profile via the Apify actor `dev_fusion/linkedin-profile-scraper`.
//! 4. Persist the scraped profile data into the user-profile memory namespace.
//!
//! Designed to run once during onboarding as a fire-and-forget enrichment
//! pass. Each stage logs progress so the caller (or a future frontend
//! progress UI) can observe what happened.

#[cfg(test)]
#[path = "linkedin_enrichment_tests.rs"]
mod tests;
include!("linkedin_enrichment_part_01.rs");
include!("linkedin_enrichment_part_02.rs");
