//! TinyMemory's host policy configuration, kept with memory composition.

/// Policy shared by the memory engine's write scrubbers.
#[must_use]
pub const fn host_policy() -> tinyredact::Policy {
    tinyredact::Policy::corroborated()
}
