use anyhow::Result;
use std::path::Path;

use super::super::Config;
use super::impl_load::{parse_config_boxed, parse_toml_off_worker};

/// Parse the text a [`ConfigSource`](super::source::ConfigSource) returned.
///
/// A file keeps its recovery: a body that does not parse falls back to the
/// `.bak` next to it, then to defaults. A document is parsed strictly and a
/// body that does not parse is an error: the file recovery would consume the
/// node-local bootstrap `config.toml.bak`, rename the bootstrap file, and save
/// stale defaults over the shared document.
pub(super) async fn parse_source_contents(
    read: &super::source::ConfigRead,
    config_path: &Path,
) -> Result<(Box<Config>, bool)> {
    let contents = read.contents.as_str();
    let read_was_recovered = read.recovered;
    // By the text's origin, not the source's kind: a document source that
    // fell back to the file (no document for the scope yet) returns file text,
    // which keeps the file's `.bak` recovery.
    if read.from_document {
        let mut config = parse_toml_off_worker(contents.to_string())
            .await
            .map_err(|error| {
                anyhow::anyhow!("the config document for this scope could not be parsed: {error}")
            })?;
        config.load_diagnostics_mut().warn_unknown_keys_once();
        *config.runtime_state.document_snapshot.lock().unwrap() =
            super::super::ConfigDocumentSnapshot {
                version: read.document_version,
                anchor: Some(read.source_anchor.clone()),
            };
        return Ok((config, false));
    }
    if read_was_recovered && contents.is_empty() {
        let mut config = super::branches::default_config_boxed();
        super::diagnostics::record_non_utf8_recovery(
            &mut config,
            config_path,
            super::ConfigLoadFallback::Defaults,
        );
        *config.runtime_state.document_snapshot.lock().unwrap() =
            super::super::ConfigDocumentSnapshot {
                version: read.document_version,
                anchor: Some(read.source_anchor.clone()),
            };
        return Ok((config, true));
    }
    let (mut config, recovered) = Box::pin(parse_config_boxed(config_path, contents)).await;
    if read_was_recovered {
        let fallback = if contents.is_empty() {
            super::ConfigLoadFallback::Defaults
        } else {
            super::ConfigLoadFallback::Backup
        };
        super::diagnostics::record_non_utf8_recovery(&mut config, config_path, fallback);
    }
    *config.runtime_state.document_snapshot.lock().unwrap() =
        super::super::ConfigDocumentSnapshot {
            version: read.document_version,
            anchor: Some(read.source_anchor.clone()),
        };
    Ok((config, recovered))
}
