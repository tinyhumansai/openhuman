//! The tinydocs module served from the linked executable, end to end.
//!
//! Local only: it needs `--features linked-modules,documents`, which no CI lane
//! or product build enables. The module is admitted through the same
//! `register_linked` seam `openhuman-core --features linked-modules` uses, and
//! `GenerateDocx` runs over the module bus with no downloaded artifact.

#[path = "support/linked_modules.rs"]
mod linked_modules;

use openhuman_core::config::schema::ModuleSourceMode;
use openhuman_core::config::Config;
use openhuman_core::modules::types::{ModuleSource, ModuleState};
use tinydocs_bus::{DocumentSection, DocumentSpec};

#[tokio::test]
async fn linked_tinydocs_generates_a_docx_without_a_release_artifact() {
    linked_modules::install();

    let mut config = Config::default();
    config.modules.enabled = true;
    // A fallback to the release must fail loudly rather than download.
    config.modules.allow_download = false;
    config.modules.source = ModuleSourceMode::Linked;

    let spec = DocumentSpec {
        title: "Linked module report".to_string(),
        author: Some("openhuman".to_string()),
        sections: vec![DocumentSection {
            heading: Some("Overview".to_string()),
            paragraphs: vec!["Served from the executable.".to_string()],
            bullets: vec!["one".to_string(), "two".to_string()],
        }],
    };

    let docx = openhuman_core::modules::documents::generate_docx(&config, &spec)
        .await
        .expect("the linked tinydocs module generates a document");
    // A .docx is a zip archive.
    assert!(docx.starts_with(b"PK"), "not a zip: {} bytes", docx.len());

    let status = openhuman_core::modules::ops::list(&config)
        .into_iter()
        .find(|module| module.id == "tinydocs")
        .expect("tinydocs is registered");
    assert_eq!(status.state, ModuleState::Ready);
    assert_eq!(status.source, Some(ModuleSource::Linked));
}
