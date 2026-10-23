//! Host adapter for scanning a workflow bundle before TinySecurity evaluates it.
//!
//! The digest binds the verdict to the exact document and resource snapshot the
//! caller will expose. Resource ordering and length-prefixing make the digest
//! unambiguous and stable across platforms.

use sha2::{Digest, Sha256};
use tinyskills::{ScanDocument, ScanReport, ScanResource, Skill, Verdict};

use crate::skills::registry::WorkflowDefinition;

/// Scan verdict and content identity for one workflow bundle snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SkillScanSnapshot {
    pub(crate) verdict: Verdict,
    pub(crate) bundle_sha256: String,
}

/// Scan the exact workflow prompt plus every discovered text resource.
///
/// Resource reads use TinySkills' path and size validation. A stale, unsafe,
/// non-text, or unreadable resource makes the scan fail closed.
pub(crate) fn scan_workflow(
    workflow: &WorkflowDefinition,
    source: Option<&Skill>,
) -> Result<SkillScanSnapshot, String> {
    let document = match &workflow.definition.system_prompt {
        tinyagents_definition::PromptSource::Inline(document) => document,
        _ => return Err("workflow scan requires an inline document".into()),
    };
    let mut resources = Vec::new();
    if let Some(source) = source {
        for path in &source.resources {
            let relative = path
                .to_str()
                .ok_or_else(|| "workflow resource path is not valid UTF-8".to_string())?;
            let text = tinyskills::read_resource(source, path)
                .map_err(|error| format!("workflow resource could not be scanned: {error}"))?;
            resources.push((relative.to_owned(), text));
        }
    }
    let borrowed = resources
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect::<Vec<_>>();
    Ok(scan_bundle_snapshot(document, &borrowed))
}

/// Return a refusal until a warning verdict has been approved by TinySecurity.
pub(crate) fn unapproved_scan_refusal(snapshot: &SkillScanSnapshot) -> Option<&'static str> {
    match snapshot.verdict {
        Verdict::Pass => None,
        Verdict::Warn => Some("workflow scan warning requires human approval"),
        Verdict::Block => Some("workflow scan blocked this bundle"),
    }
}

/// Reject blocked bundles and bind any approval to the precise scanned bytes.
pub(crate) fn scan_approval_refusal(
    snapshot: &SkillScanSnapshot,
    approved_digest: Option<&str>,
) -> Option<&'static str> {
    if snapshot.verdict == Verdict::Block {
        return Some("workflow scan blocked this bundle");
    }
    if approved_digest.is_some_and(|digest| digest != snapshot.bundle_sha256) {
        return Some("workflow bundle changed after security approval");
    }
    if snapshot.verdict == Verdict::Warn && approved_digest.is_none() {
        return Some("workflow scan warning requires human approval");
    }
    None
}

/// Scan the document and resources and bind the result to their exact content.
pub(crate) fn scan_bundle_snapshot(
    document: &str,
    resources: &[(&str, &str)],
) -> SkillScanSnapshot {
    let mut resources = resources
        .iter()
        .map(|(path, text)| ScanResource {
            path: (*path).to_owned(),
            text: (*text).to_owned(),
        })
        .collect::<Vec<_>>();
    resources.sort_by(|left, right| left.path.cmp(&right.path));

    let report: ScanReport = tinyskills::scan_skill(
        &ScanDocument {
            name: "",
            description: "",
            category: None,
            version: None,
            body: document,
            extra_frontmatter: &[],
        },
        &resources,
    );

    let mut hasher = Sha256::new();
    hasher.update(b"openhuman.skill-bundle.v1\0");
    hash_field(&mut hasher, b"document");
    hash_field(&mut hasher, document.as_bytes());
    for resource in &resources {
        hash_field(&mut hasher, b"resource-path");
        hash_field(&mut hasher, resource.path.as_bytes());
        hash_field(&mut hasher, b"resource-content");
        hash_field(&mut hasher, resource.text.as_bytes());
    }

    SkillScanSnapshot {
        verdict: report.verdict(),
        bundle_sha256: hex::encode(hasher.finalize()),
    }
}

fn hash_field(hasher: &mut Sha256, field: &[u8]) {
    hasher.update((field.len() as u64).to_be_bytes());
    hasher.update(field);
}
