//! Tests for the document call client.
//!
//! Nothing here loads a module. What is testable without one is the part that
//! decides what a tool does next: how a bus failure is classified, and that the
//! unavailable path is reached without a broker. The round trips themselves are
//! covered where they can be honest — `tinydocs`' own loader E2E, which drives a
//! real module over a real broker.

use super::{classify, sha256_hex, DocumentCallError};
use crate::openhuman::config::Config;
use tinydocs::spec::{DocumentSpec, WirePresentationSpec};

/// A config with modules enabled but nothing fetchable.
fn offline_config() -> Config {
    let mut config = Config::default();
    config.modules.enabled = true;
    config.modules.allow_download = false;
    config
}

/// A bus failure carrying `name`.
fn failure(name: &str) -> tinybus::Error {
    tinybus::Error::MethodFailed {
        name: name.to_string(),
        message: "something went wrong".to_string(),
    }
}

#[test]
fn an_invalid_input_is_reported_as_something_a_model_can_fix() {
    assert!(matches!(
        classify(&failure("ai.tinyhumans.tinydocs.Error.InvalidInput")),
        DocumentCallError::InvalidInput(_)
    ));
}

#[test]
fn generation_and_extraction_failures_are_not_input_errors() {
    // Telling a model its spec was wrong when the writer broke sends it into a
    // rewrite loop over a spec that was fine.
    for name in [
        "ai.tinyhumans.tinydocs.Error.GenerationFailed",
        "ai.tinyhumans.tinydocs.Error.ExtractionFailed",
        "ai.tinyhumans.tinydocs.Error.ModuleFailed",
        "ai.tinyhumans.tinydocs.Error.TransferFailed",
        "ai.tinyhumans.tinydocs.Error.OutputRefused",
        "ai.tinyhumans.tinydocs.Error.UnknownOutput",
    ] {
        assert!(
            matches!(classify(&failure(name)), DocumentCallError::Failed(_)),
            "{name} should not be reported as an input error"
        );
    }
}

#[test]
fn an_unrecognised_wire_name_is_a_failure_not_an_input_error() {
    // The conservative direction: a name this build does not know about is more
    // likely a newer module than a bad spec.
    assert!(matches!(
        classify(&failure("ai.tinyhumans.tinydocs.Error.SomethingNewer")),
        DocumentCallError::Failed(_)
    ));
}

#[test]
fn a_missing_module_reads_as_unavailable() {
    assert!(matches!(
        classify(&failure("ai.tinyhumans.tinybus.Error.ModuleUnavailable")),
        DocumentCallError::Unavailable(_)
    ));
}

#[test]
fn every_error_renders_as_its_message() {
    for error in [
        DocumentCallError::Unavailable("gone".to_string()),
        DocumentCallError::InvalidInput("bad title".to_string()),
        DocumentCallError::Failed("writer stopped".to_string()),
    ] {
        assert!(!error.to_string().is_empty());
    }
    assert_eq!(
        DocumentCallError::InvalidInput("bad title".to_string()).to_string(),
        "bad title"
    );
}

#[test]
fn the_digest_matches_the_modules_own_vector() {
    // Both sides compute this independently; if they disagree every document
    // round trip fails its integrity check.
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[tokio::test]
async fn a_disabled_host_reports_unavailable_without_starting_a_broker() {
    let mut config = offline_config();
    config.modules.enabled = false;

    let spec = DocumentSpec {
        title: "Charter".to_string(),
        author: None,
        sections: vec![],
    };
    assert!(matches!(
        super::generate_docx(&config, &spec).await,
        Err(DocumentCallError::Unavailable(_))
    ));

    let deck = WirePresentationSpec {
        title: "Deck".to_string(),
        author: None,
        theme: None,
        slides: vec![],
    };
    assert!(matches!(
        super::generate_pptx(&config, &deck, &[]).await,
        Err(DocumentCallError::Unavailable(_))
    ));

    assert!(matches!(
        super::extract_text(&config, b"%PDF-1.4\n").await,
        Err(DocumentCallError::Unavailable(_))
    ));
}
