# proc_metrics

Cross-platform process memory sampling, written for the `rss-bench` benchmark
harness (#5046) but usable by any caller wanting this process's RSS / peak-RSS
figures.

## Key files

| File | Role |
| --- | --- |
| `crates/openhuman-core/src/platform/proc_metrics/mod.rs` | `ProcSample`, `sample_self`, `/proc` parsers, and roster/report aggregation. |
| `crates/openhuman-core/src/platform/proc_metrics/tree.rs` | `sample_tree`, `ChildSample`, `TreeSample`: process-*tree* RSS including descendants. |

## `ProcSample`

One sample of the current process: `rss_kib`, `pss_kib` (Linux only),
`private_clean_kib` / `private_dirty_kib` (Linux only), `vm_hwm_kib` (peak
RSS), `threads`, `binary_size_bytes`, `cpu_user_ms` / `cpu_system_ms`, and
`open_fds` (`None` when the platform lookup is unavailable, never a
misleading zero).

`sample_self` supports Linux (`/proc/self/status` + `/proc/self/smaps_rollup`
+ `/proc/self/stat`) and macOS (`proc_pidinfo` / `proc_pid_rusage`); it
returns a structured `anyhow::Result` error elsewhere rather than fabricating
a reading. `parse_status` and `parse_smaps_rollup` are OS-agnostic, take
`&str`, and are unit-tested against literal `/proc` text without a live
filesystem.

## Aggregation

`RosterResult::from_samples` reduces repeated fresh-process samples into
median/min/max/mean RSS, median PSS, peak `VmHWM`, and median thread count.
`BenchReport` bundles `RosterResult`s per roster size plus the git SHA and
kernel string for the CI JSON artifact; `per_agent_increment_kib` derives the
marginal per-agent RSS cost from the smallest and largest rosters measured.

`RSS_BUDGET_KIB` (20 MiB) and `RSS_HARD_CAP_KIB` (30 MiB) are the product
budget and CI-gate ceiling for the embedded agent roster (#5046).

## Process trees

`sample_tree` (in [`tree.rs`](./tree.rs)) measures a process and all of its descendants,
the interpreter children a skill run or shell tool spawns, returning a
`TreeSample` (`self_sample`, `children: Vec<ChildSample>`, `tree_rss_kib`).
Descendant lookups that fail (a child that raced away, a permission error)
are skipped with a stderr note rather than aborting the sample.

## Used by

- `profile/src/bin/rss_bench.rs` and `profile/src/bin/library_profile/` in
  [openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks) (feature `rss-bench`): the RSS/memory
  benchmark binaries this module was built for.
- [`profile/docs/library-benchmarking.md`](https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/profile/docs/library-benchmarking.md)
 : documents the benchmark methodology and points readers here for the
  sampling implementation.

## Further reading

- [Parent module (`platform`)](../README.md)
- [Platform and availability](../../../../../docs/gitbooks/en/features/platform.md)
- [Tauri shell architecture](../../../../../docs/gitbooks/en/developing/architecture/tauri-shell.md)
- [Architecture overview](../../../../../docs/gitbooks/en/developing/architecture.md)
