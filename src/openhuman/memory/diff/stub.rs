//! The `memory-git`-disabled controller surface of `memory::diff`.
//!
//! The *ops* mirror lives with the domain in `tinymemory_core::diff::stub` —
//! it is the feature-off arm of core code, and always-on core callers
//! (`sources::sync`) reach it without any feature awareness, which is the whole
//! point of a stub.
//!
//! What stays here is the registration half. Registration sites want
//! **absence**: the aggregators return empty vecs so the `memory_diff.*`
//! controllers become unknown-method rather than known-and-always-failing.

/// No controllers: the `memory_diff` namespace answers unknown-method.
///
/// Empty rather than a set of always-erroring handlers, so `/schema` does not
/// advertise a surface this build cannot serve.
pub fn all_memory_diff_controller_schemas() -> Vec<crate::core::ControllerSchema> {
    Vec::new()
}

/// No controllers to register. See [`all_memory_diff_controller_schemas`].
pub fn all_memory_diff_registered_controllers() -> Vec<crate::core::all::RegisteredController> {
    Vec::new()
}
