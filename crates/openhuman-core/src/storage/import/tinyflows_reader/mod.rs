//! Host-owned read-only decoders for TinyFlows' retired SQLite and JSONL
//! formats. The returned values are bus DTOs; this module contains no owner
//! storage implementation and writes only to its private SQLite snapshots.

mod checkpoint_reader;
mod flow_reader;
mod legacy_cron;
mod legacy_db;
mod report;

pub(super) use checkpoint_reader::read_legacy_checkpoint_batches;
pub(super) use flow_reader::read_legacy_flow_catalog;
pub(super) use legacy_cron::read_legacy_cron;
pub(super) use report::ImportReport;
