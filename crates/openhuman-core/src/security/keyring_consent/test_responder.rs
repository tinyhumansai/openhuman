//! Answers the credential store's keyring prompts in test builds.
//!
//! Every keyring operation TinySecurity performs waits for the user to
//! approve it. Tests that save credentials have no user, so each save would
//! wait out the whole consent window. This module is compiled only into test
//! builds, `e2e-test-support` builds and the integration tests'
//! `test-keyring-consent` builds, never a product build.
//!
//! `OPENHUMAN_TEST_KEYRING_CONSENT` selects the answer: `approve`, `deny`, or
//! `off` to leave prompts unanswered. Unit tests approve by default; the
//! other builds answer only when the variable says so. Only the
//! credential store's prompts are answered, so a test of an unanswered
//! prompt raises it under another caller.

/// The caller TinySecurity reports for the credential store's keyring use.
const CREDENTIAL_STORE: &str = "openhuman.credentials";

/// The answer for a prompt raised by `caller_agent`, if this build answers it.
pub(super) fn decision(caller_agent: &str) -> Option<bool> {
    if caller_agent != CREDENTIAL_STORE {
        return None;
    }
    match std::env::var("OPENHUMAN_TEST_KEYRING_CONSENT").as_deref() {
        Ok("approve") => Some(true),
        Ok("deny") => Some(false),
        Ok(_) => None,
        Err(_) => cfg!(test).then_some(true),
    }
}
