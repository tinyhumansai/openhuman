//! Memory document conversion through the TinyDocs module.
//!
//! Brain ingest and file-backed sources share this chain. Textual formats stay
//! with TinyMemory's native converter; PDF, DOCX, PPTX and XLSX are extracted
//! by the configured TinyDocs module, with no host-side parser fallback.

use crate::config::Config;
use tinymemory_integrations::documents::ConverterChain;

#[cfg(feature = "documents")]
use async_trait::async_trait;
#[cfg(feature = "documents")]
use tinymemory_integrations::documents::{
    ConvertedDocument, DocumentConverter, DocumentFormat, Error, RawDocument, Result,
};

/// The bus-backed document extraction adapter. Its name and metadata stay
/// compatible with the former OfficeConverter while parsing belongs to TinyDocs.
#[cfg(feature = "documents")]
struct TinyDocsConverter {
    config: Config,
}

#[cfg(feature = "documents")]
#[async_trait]
impl DocumentConverter for TinyDocsConverter {
    fn name(&self) -> &str {
        "office"
    }

    fn supports(&self, format: DocumentFormat) -> bool {
        matches!(
            format,
            DocumentFormat::Pdf
                | DocumentFormat::Docx
                | DocumentFormat::Pptx
                | DocumentFormat::Xlsx
        )
    }

    async fn convert(&self, document: &RawDocument) -> Result<ConvertedDocument> {
        let format = document.format();
        let wire_format = match format {
            DocumentFormat::Pdf => tinydocs_bus::DocumentFormat::Pdf,
            DocumentFormat::Docx => tinydocs_bus::DocumentFormat::Docx,
            DocumentFormat::Pptx => tinydocs_bus::DocumentFormat::Pptx,
            DocumentFormat::Xlsx => tinydocs_bus::DocumentFormat::Xlsx,
            _ => unreachable!("supports limits formats to TinyDocs formats"),
        };
        let markdown =
            crate::modules::documents::convert_markdown(&self.config, wire_format, &document.bytes)
                .await
                .map_err(map_module_error)?;

        Ok(
            ConvertedDocument::new(markdown, format, document.bytes.len())
                .with_metadata(serde_json::json!({ "converter": self.name() })),
        )
    }
}

#[cfg(feature = "documents")]
fn map_module_error(error: crate::modules::documents::DocumentCallError) -> Error {
    use crate::modules::documents::DocumentCallError;
    match error {
        DocumentCallError::InvalidInput(message) => Error::Invalid(message),
        DocumentCallError::Unavailable(message) | DocumentCallError::Failed(message) => {
            Error::Converter {
                converter: "tinydocs".to_string(),
                message,
            }
        }
    }
}

/// Build the host's format-first converter chain against this config snapshot.
pub(crate) fn converter(config: &Config) -> ConverterChain {
    let chain = ConverterChain::default();
    #[cfg(feature = "documents")]
    let chain = chain.prepend(Box::new(TinyDocsConverter {
        config: config.clone(),
    }));
    #[cfg(not(feature = "documents"))]
    let _ = config;
    chain
}

#[cfg(test)]
#[path = "convert_tests.rs"]
mod tests;
