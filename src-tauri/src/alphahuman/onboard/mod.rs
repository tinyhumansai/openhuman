//! Onboarding helpers for Alphahuman.

pub mod models;

pub use models::{
    run_models_refresh, ModelCacheSnapshot, ModelRefreshResult, ModelRefreshSource,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_reexport_exists<F>(_value: F) {}

    #[test]
    fn reexports_models_refresh() {
        assert_reexport_exists(run_models_refresh);
        assert_reexport_exists(ModelRefreshResult::default);
    }
}
