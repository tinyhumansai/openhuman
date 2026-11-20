//! `SecurityPolicy` as the harness tools' gates.
//!
//! The filesystem and network tools live in `tinyagents_harness_tools`
//! (`fs::filesystem`, `net::network`) and ask a gate trait every policy
//! question. [`fs`] implements `FsGate` and [`net`] implements `NetGate` (plus
//! the per-turn [`net::RoutedGate`]) for [`super::SecurityPolicy`], so the
//! autonomy tier, TinySecurity's protected-path floor, `workspace_only`,
//! trusted roots, the privacy-mode egress gate and the approval gate remain
//! host decisions. The tools only ask. Registration is in `tools/ops.rs`; the
//! network tools' host wiring is `tools/impl/network.rs`.

mod fs;
mod net;

pub(crate) use net::RoutedGate;
