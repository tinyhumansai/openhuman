//! Business logic for the `flows::` domain: validate-on-save CRUD plus the
//! end-to-end `flows_run` / `flows_resume` path. Delegated to from
//! `schemas.rs`'s `handle_*` RPC/CLI handlers, mirroring
//! `src/openhuman/cron/ops.rs`.

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
include!("ops_part_01.rs");
include!("ops_part_02.rs");
include!("ops_part_03.rs");
include!("ops_part_04.rs");
include!("ops_part_05.rs");
include!("ops_part_06.rs");
include!("ops_part_07.rs");
include!("ops_part_08.rs");
include!("ops_part_09.rs");
include!("ops_part_10.rs");
include!("ops_part_11.rs");
include!("ops_part_12.rs");
