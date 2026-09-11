//! A recording fake [`MemoryProvider`] for the guard's tests.
//!
//! `NullMemoryProvider` cannot serve here: it advertises only the mandatory
//! three and returns `None` from every `as_*` accessor, so a guard built over
//! it would have no family decorators at all — which is precisely what the
//! interesting tests are about. This fake implements **all thirteen** families
//! and records what actually reached it, so a test can assert both "the driver
//! saw the value the guard rewrote" and "the driver saw nothing at all".

#![cfg(test)]

include!("test_support_part_01.rs");
include!("test_support_part_02.rs");
include!("test_support_part_03.rs");
