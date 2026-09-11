//! Tests for the document call client.
//!
//! Nothing here loads a module. What is testable without one is the part that
//! decides what a tool does next: how a bus failure is classified, and that the
//! unavailable path is reached without a broker. The round trips themselves are
//! covered where they can be honest — `tinydocs`' own loader E2E, which drives a
//! real module over a real broker.

use super::{classify, sha256_hex, DocumentCallError};
use crate::openhuman::config::Config;
use crate::openhuman::tools::implementations::document::format::spec::{
    DocumentSpec, WirePresentationSpec,
};

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

#[test]
fn the_registry_entry_matches_the_interface_this_client_calls() {
    // The registry is a plain `const` table and cannot name a gated crate, so
    // the bus name and object path are written out there by hand. This is what
    // checks them against the contract's own constants — a mismatch is not a
    // compile error, it is a `NameHasNoOwner` at first use, in the field, on
    // whichever platform nobody tested.
    let record =
        crate::openhuman::modules::registry::find("tinydocs").expect("tinydocs is registered");
    assert_eq!(record.bus_name, tinydocs_bus::names::BUS_NAME);
    assert_eq!(record.object_path, tinydocs_bus::names::OBJECT_PATH);
}

#[test]
fn every_member_this_client_calls_is_one_the_contract_declares() {
    // The five calls in this module are written as `tinydocs_bus` constants, so
    // a rename upstream is a compile error here rather than a `MemberNotFound`
    // at runtime. This pins the other direction: that the constants are the
    // contract's whole surface, so a member added upstream shows up as an
    // unused one here rather than being quietly unreachable.
    use tinydocs_bus::names::methods;
    let called = [
        methods::GENERATE_DOCX,
        methods::GENERATE_PPTX,
        methods::EXTRACT_TEXT,
        methods::READ_OUTPUT,
        methods::RELEASE_OUTPUT,
    ];
    for member in tinydocs_bus::names::METHODS {
        assert!(
            called.contains(&member),
            "the contract declares `{member}`, which this client never calls"
        );
    }
}
