use std::collections::BTreeSet;
use std::sync::OnceLock;

use super::types::{Capability, CapabilityCategory};
#[cfg(test)]
use super::types::{CapabilityStatus, PrivacyDataKind};

#[path = "catalog_data.rs"]
mod catalog_data;
use catalog_data::CAPABILITIES;

static VALIDATED: OnceLock<()> = OnceLock::new();

pub fn all_capabilities() -> &'static [Capability] {
    ensure_validated();
    CAPABILITIES
}

pub fn capabilities_by_category(category: CapabilityCategory) -> Vec<Capability> {
    ensure_validated();
    CAPABILITIES
        .iter()
        .filter(|capability| capability.category == category)
        .copied()
        .collect()
}

pub fn lookup(id: &str) -> Option<Capability> {
    ensure_validated();
    let normalized = id.trim();
    CAPABILITIES
        .iter()
        .find(|capability| capability.id == normalized)
        .copied()
}

pub fn search(query: &str) -> Vec<Capability> {
    ensure_validated();
    let normalized = query.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return CAPABILITIES.to_vec();
    }

    CAPABILITIES
        .iter()
        .filter(|capability| searchable_text(capability).contains(&normalized))
        .copied()
        .collect()
}

fn searchable_text(capability: &Capability) -> String {
    format!(
        "{} {} {} {} {} {} {}",
        capability.id,
        capability.name,
        capability.domain,
        capability.category.as_str(),
        capability.description,
        capability.how_to,
        capability.status.as_str()
    )
    .to_ascii_lowercase()
}

fn ensure_validated() {
    VALIDATED.get_or_init(|| {
        let mut ids = BTreeSet::new();
        for capability in CAPABILITIES {
            assert!(
                !capability.id.trim().is_empty(),
                "about_app capability id must not be empty"
            );
            assert!(
                ids.insert(capability.id),
                "duplicate about_app capability id '{}'",
                capability.id
            );
        }

        tracing::debug!(
            count = CAPABILITIES.len(),
            "[about_app] validated capability catalog"
        );
    });
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
