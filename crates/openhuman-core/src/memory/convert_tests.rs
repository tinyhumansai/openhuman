use super::*;

use crate::config::Config;
use tinymemory_integrations::documents::{DocumentConverter, Error, RawDocument};
#[cfg(feature = "documents")]
use {
    crate::memory::brain::{ingest, BrainIngestParams},
    crate::memory::test_fixtures::{bind_reference, config_in, stored},
    tinymemory_api::{ItemKind, MetaFilter},
};

/// A one-page PDF whose only text is `text` (Helvetica, no compression).
fn pdf_saying(text: &str) -> Vec<u8> {
    let stream = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_string(),
        format!(
            "<< /Length {} >>\nstream\n{stream}\nendstream",
            stream.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.push_str(&format!("{} 0 obj\n{object}\nendobj\n", i + 1));
    }
    let xref = out.len();
    out.push_str(&format!(
        "xref\n0 {}\n0000000000 65535 f \n",
        objects.len() + 1
    ));
    for offset in offsets {
        out.push_str(&format!("{offset:010} 00000 n \n"));
    }
    out.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    ));
    out.into_bytes()
}

/// A valid PDF with one text-bearing page per supplied string.
fn pdf_pages_saying(texts: &[String]) -> Vec<u8> {
    let mut objects = Vec::with_capacity(3 + texts.len() * 2);
    objects.push("<< /Type /Catalog /Pages 2 0 R >>".to_string());
    let kids = (0..texts.len())
        .map(|index| format!("{} 0 R", 4 + index * 2))
        .collect::<Vec<_>>()
        .join(" ");
    objects.push(format!(
        "<< /Type /Pages /Kids [{kids}] /Count {} >>",
        texts.len()
    ));
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string());
    for (index, text) in texts.iter().enumerate() {
        let content_object = 5 + index * 2;
        let stream = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {content_object} 0 R \
             /Resources << /Font << /F1 3 0 R >> >> >>"
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{stream}\nendstream",
            stream.len()
        ));
    }
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.push_str(&format!("{} 0 obj\n{object}\nendobj\n", index + 1));
    }
    let xref = out.len();
    out.push_str(&format!(
        "xref\n0 {}\n0000000000 65535 f \n",
        objects.len() + 1
    ));
    for offset in offsets {
        out.push_str(&format!("{offset:010} 00000 n \n"));
    }
    out.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    ));
    out.into_bytes()
}

#[cfg(feature = "documents")]
fn office_archive(parts: &[(&str, &str)]) -> Vec<u8> {
    use std::io::Write;
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, contents) in parts {
        archive.start_file(*name, options).unwrap();
        archive.write_all(contents.as_bytes()).unwrap();
    }
    archive.finish().unwrap().into_inner()
}

#[cfg(feature = "documents")]
fn docx_saying(text: &str) -> Vec<u8> {
    let xml = format!(
        "<w:document xmlns:w='w'><w:body><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:body></w:document>"
    );
    office_archive(&[("word/document.xml", &xml)])
}

#[cfg(feature = "documents")]
fn pptx_saying(text: &str) -> Vec<u8> {
    let slide =
        format!("<p:sld xmlns:p='p' xmlns:a='a'><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:sld>");
    office_archive(&[
        (
            "ppt/presentation.xml",
            "<p:presentation xmlns:p='p' xmlns:r='r'><p:sldIdLst><p:sldId r:id='r1'/></p:sldIdLst></p:presentation>",
        ),
        (
            "ppt/_rels/presentation.xml.rels",
            "<Relationships><Relationship Id='r1' Type='http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide' Target='slides/slide1.xml'/></Relationships>",
        ),
        ("ppt/slides/slide1.xml", &slide),
    ])
}

#[cfg(feature = "documents")]
fn xlsx_saying(text: &str) -> Vec<u8> {
    let content_types = "<Types xmlns='http://schemas.openxmlformats.org/package/2006/content-types'><Default Extension='rels' ContentType='application/vnd.openxmlformats-package.relationships+xml'/><Default Extension='xml' ContentType='application/xml'/><Override PartName='/xl/workbook.xml' ContentType='application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml'/><Override PartName='/xl/worksheets/sheet1.xml' ContentType='application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml'/></Types>";
    let root_rels = "<Relationships xmlns='http://schemas.openxmlformats.org/package/2006/relationships'><Relationship Id='rId1' Type='http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument' Target='xl/workbook.xml'/></Relationships>";
    let workbook = "<workbook xmlns='http://schemas.openxmlformats.org/spreadsheetml/2006/main' xmlns:r='http://schemas.openxmlformats.org/officeDocument/2006/relationships'><sheets><sheet name='Sheet1' sheetId='1' r:id='rId1'/></sheets></workbook>";
    let workbook_rels = "<Relationships xmlns='http://schemas.openxmlformats.org/package/2006/relationships'><Relationship Id='rId1' Type='http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet' Target='worksheets/sheet1.xml'/></Relationships>";
    let sheet = format!("<worksheet xmlns='http://schemas.openxmlformats.org/spreadsheetml/2006/main'><sheetData><row r='1'><c r='A1' t='inlineStr'><is><t>{text}</t></is></c></row></sheetData></worksheet>");
    office_archive(&[
        ("[Content_Types].xml", content_types),
        ("_rels/.rels", root_rels),
        ("xl/workbook.xml", workbook),
        ("xl/_rels/workbook.xml.rels", workbook_rels),
        ("xl/worksheets/sheet1.xml", &sheet),
    ])
}

#[cfg(feature = "documents")]
fn local_module_config() -> Config {
    let mut config = Config::default();
    config.modules.allow_download = false;
    config
}

#[cfg(feature = "documents")]
#[tokio::test]
#[ignore = "needs a built TinyDocs module and its own process; set OPENHUMAN_MODULE_PATH"]
async fn a_pdf_converts_to_its_text() {
    let raw = RawDocument::new(pdf_saying("The vendor code is PV-7023")).with_filename("brief.pdf");
    let converted = converter(&local_module_config())
        .convert(&raw)
        .await
        .expect("the office converter reads pdf");
    assert!(
        converted.markdown.contains("PV-7023"),
        "{}",
        converted.markdown
    );
    assert_eq!(converted.source_bytes, raw.bytes.len());
    assert_eq!(converted.metadata["converter"], "office");
}

#[cfg(feature = "documents")]
#[tokio::test]
#[ignore = "needs the released TinyDocs module in OPENHUMAN_MODULE_PATH and its own process"]
async fn the_released_module_extracts_all_memory_office_formats() {
    let config = local_module_config();
    let cases = [
        (
            RawDocument::new(pdf_saying("receipt PDF text")).with_filename("receipt.pdf"),
            "receipt PDF text",
        ),
        (
            RawDocument::new(docx_saying("word text")).with_filename("letter.docx"),
            "word text",
        ),
        (
            RawDocument::new(pptx_saying("slide text")).with_filename("deck.pptx"),
            "slide text",
        ),
        (
            RawDocument::new(xlsx_saying("cell text")).with_filename("sheet.xlsx"),
            "cell text",
        ),
    ];
    for (raw, expected) in cases {
        let converted = converter(&config)
            .convert(&raw)
            .await
            .unwrap_or_else(|error| panic!("{} extraction failed: {error}", raw.display_name()));
        assert!(
            converted.markdown.contains(expected),
            "{}",
            converted.markdown
        );
        assert_eq!(converted.source_bytes, raw.bytes.len());
        assert_eq!(converted.metadata["converter"], "office");
        if raw.display_name() == "sheet.xlsx" {
            assert_eq!(converted.markdown, "Sheet1 | cell text");
        }
    }

    let large_docx = "complete paragraph ".repeat(12_500);
    let raw = RawDocument::new(docx_saying(&large_docx)).with_filename("long.docx");
    let bounded_preview = crate::modules::documents::extract_document(
        &config,
        &raw.bytes,
        &tinydocs_bus::ExtractDocumentSpec {
            format: tinydocs_bus::DocumentFormat::Docx,
            max_text_bytes: 200_000,
            max_sections: 256,
        },
    )
    .await
    .expect("the preview operation remains bounded");
    assert!(bounded_preview.truncated);
    let converted = converter(&config)
        .convert(&raw)
        .await
        .expect("text beyond the intake preview limit remains complete");
    assert!(converted.markdown.len() > 200_000);
    let normalized = large_docx.split_whitespace().collect::<Vec<_>>().join(" ");
    assert_eq!(converted.markdown, normalized);

    let pages = (1..=300)
        .map(|page| format!("Page {page} content"))
        .collect::<Vec<_>>();
    let raw = RawDocument::new(pdf_pages_saying(&pages)).with_filename("many-pages.pdf");
    let bounded_preview = crate::modules::documents::extract_document(
        &config,
        &raw.bytes,
        &tinydocs_bus::ExtractDocumentSpec {
            format: tinydocs_bus::DocumentFormat::Pdf,
            max_text_bytes: 200_000,
            max_sections: 256,
        },
    )
    .await
    .expect("the preview operation remains bounded");
    assert!(bounded_preview.truncated);
    assert_eq!(bounded_preview.section_count, 300);
    assert_eq!(bounded_preview.sections.len(), 256);
    let converted = converter(&config)
        .convert(&raw)
        .await
        .expect("page counts beyond the intake section limit remain complete");
    assert!(converted.markdown.contains("Page 300 content"));
    assert_eq!(converted.markdown.matches('\u{c}').count(), 299);
}

#[tokio::test]
async fn markdown_still_goes_through_the_native_converter() {
    let raw = RawDocument::new(b"# Note\n\nPlain markdown.".to_vec()).with_filename("note.md");
    let converted = converter(&Config::default())
        .convert(&raw)
        .await
        .expect("native handles markdown");
    assert!(converted.markdown.contains("Plain markdown."));
}

#[tokio::test]
async fn an_unknown_binary_is_still_refused_with_a_clear_error() {
    let raw = RawDocument::new(vec![0u8, 1, 2, 3, 255]).with_filename("blob.bin");
    let error = converter(&Config::default())
        .convert(&raw)
        .await
        .unwrap_err();
    assert!(matches!(error, Error::UnsupportedFormat(_)), "{error}");
}

#[cfg(feature = "documents")]
#[tokio::test]
#[ignore = "needs a built TinyDocs module and its own process; set OPENHUMAN_MODULE_PATH"]
async fn brain_ingest_files_a_pdf_by_path() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let mut config = config;
    config.modules.allow_download = false;
    let engine = bind_reference(&config);
    let path = tmp.path().join("brief.pdf");
    std::fs::write(&path, pdf_saying("The vendor code is PV-7023")).unwrap();

    let view = ingest(
        &config,
        BrainIngestParams {
            path: Some(path.display().to_string()),
            text: None,
            source: None,
            title: None,
        },
    )
    .await
    .expect("a pdf is ingested, not refused");
    assert_eq!(view.source, "files", "a PDF is a file like any other");

    let docs = stored(&engine, MetaFilter::kinds([ItemKind::Document])).await;
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].meta.namespace.to_string(), "source:files");
    assert!(docs[0].text.contains("PV-7023"), "{}", docs[0].text);
}

/// Without `documents` the office parsers are not linked: a PDF is refused
/// as unsupported, never mis-read as text.
#[cfg(not(feature = "documents"))]
#[tokio::test]
async fn without_the_documents_feature_a_pdf_is_refused_cleanly() {
    let raw = RawDocument::new(pdf_saying("The vendor code is PV-7023")).with_filename("brief.pdf");
    let error = converter(&Config::default())
        .convert(&raw)
        .await
        .unwrap_err();
    assert!(matches!(error, Error::UnsupportedFormat(_)), "{error}");
}

#[cfg(feature = "documents")]
#[tokio::test]
async fn a_pdf_does_not_fall_back_to_a_local_parser_when_modules_are_disabled() {
    let raw = RawDocument::new(pdf_saying("The vendor code is PV-7023")).with_filename("brief.pdf");
    let mut config = Config::default();
    config.modules.enabled = false;

    let error = converter(&config).convert(&raw).await.unwrap_err();
    assert!(
        matches!(error, Error::Converter { ref converter, .. } if converter == "tinydocs"),
        "expected a TinyDocs-unavailable converter error, got {error}"
    );
}

#[cfg(feature = "documents")]
#[tokio::test]
async fn memory_module_intake_keeps_the_32_mib_limit() {
    let raw = RawDocument::new(vec![
        0;
        tinymemory_integrations::documents::MAX_DOCUMENT_BYTES
            + 1
    ])
    .with_filename("oversized.pdf");
    let mut config = Config::default();
    config.modules.enabled = false;

    let error = converter(&config).convert(&raw).await.unwrap_err();
    assert!(
        matches!(error, Error::TooLarge { size, limit }
            if size == tinymemory_integrations::documents::MAX_DOCUMENT_BYTES + 1
                && limit == tinymemory_integrations::documents::MAX_DOCUMENT_BYTES),
        "the 32 MiB guard must run before module loading, got {error}"
    );
}

#[cfg(feature = "documents")]
#[test]
fn the_office_converter_names_itself_and_claims_only_office_formats() {
    use tinymemory_integrations::documents::DocumentFormat;
    let office = TinyDocsConverter {
        config: Config::default(),
    };
    assert_eq!(office.name(), "office");
    assert!(office.supports(DocumentFormat::Pdf));
    assert!(office.supports(DocumentFormat::Docx));
    assert!(office.supports(DocumentFormat::Pptx));
    assert!(office.supports(DocumentFormat::Xlsx));
    assert!(!office.supports(DocumentFormat::Markdown));
}
