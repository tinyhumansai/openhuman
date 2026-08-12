//! Controller-surface tests for the people domain.
//!
//! Moved here with `schemas.rs` when the memory subsystem was extracted: the
//! controller list is host surface, so the test that pins its shape belongs on
//! this side of the seam.

#[cfg(test)]
mod tests {
    /// Verify that the schema exposes four controllers now that
    /// `refresh_address_book` is wired up.
    #[test]
    fn schema_exposes_four_controllers() {
        use crate::openhuman::memory::people::schemas;
        let names: Vec<_> = schemas::all_controller_schemas()
            .into_iter()
            .map(|s| s.function)
            .collect();
        assert!(
            names.contains(&"refresh_address_book"),
            "missing refresh_address_book: {names:?}"
        );
        assert_eq!(names.len(), 4);
    }
}
