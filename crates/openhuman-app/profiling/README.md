# profiling

An offline developer tool that measures CPU and memory of a running OpenHuman
desktop app. It is a standalone Cargo crate (package
`openhuman-tauri-resource-profiler`) with its own `[workspace]` table and
`Cargo.lock`. It depends only on `serde`, `serde_json` and `sysinfo`, and it is
not linked into, registered with, or shipped in the app, so profiling never
changes the shipped dependency graph.

## How it works

```text
pnpm profile:tauri --pid <PID> --duration 15
   |
   | cargo run --manifest-path crates/openhuman-app/profiling/Cargo.toml --
   v
parse_args
   |
   +--> (macOS, unless --no-stacks) /usr/bin/sample <PID> <secs> 10 -mayDie
   |        -> cpu-stacks.txt, runs alongside the sampling loop
   |
   +--> capture_samples: every --interval-ms (min: sysinfo's CPU interval)
   |        refresh all processes, keep the host PID and its descendants,
   |        classify each one (classify_process), sum CPU and RSS per group
   |
   v
build_report -> mean and peak per group
             -> CPU samples per Rust module parsed from cpu-stacks.txt
   |
   v
<out>/resources.json   raw time series plus mean and peak
<out>/resources.md     summary table (also printed to stdout)
<out>/cpu-stacks.txt   macOS sample report, when captured
```

The tool fails if the PID is not running at the start or exits during the
capture.

### Attribution

The Rust core runs inside the Tauri host process, so operating-system metrics
cannot separate the shell from `openhuman_core` or assign heap pages to Rust
modules. The profiler reports that process as one row, "Tauri host + embedded
Rust core".

To split host CPU by module, it reads the recursive sample counts in
`cpu-stacks.txt` and groups symbols by their first path segment under
`openhuman_core::` (for example `openhuman_core::agent`) or `openhuman::` (the
shell library, for example `openhuman::core_process`). Stack capture is
automatic only on macOS.

CPU percentages are per logical CPU and can exceed 100 when a component uses
several cores. RAM is resident memory as reported by `sysinfo`.

## Usage

Start the app from the current checkout:

```bash
pnpm dev:app
```

Find the main host PID (on macOS the binary is
`OpenHuman.app/Contents/MacOS/OpenHuman`):

```bash
pgrep -fl '/OpenHuman$'
```

Capture a representative workload:

```bash
pnpm profile:tauri --pid <PID> --duration 15
```

| Flag | Default | Meaning |
| --- | --- | --- |
| `--pid PID` | required | The main app process, not a webview helper. |
| `--duration SECONDS` | 15 | Capture length. Must be greater than zero. |
| `--interval-ms MS` | 250 | Sample interval, raised to `sysinfo`'s minimum CPU refresh interval if lower. |
| `--out PATH` | `target/profile/tauri-resources-<unix-seconds>` | Output directory, relative to the working directory. |
| `--stacks`, `--no-stacks` | on for macOS | Turn the `/usr/bin/sample` capture on or off. |

For the smaller embedded-core-only Linux RSS and PSS benchmark, use the
`rss-bench` binary in the `profile/` crate of
[openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks) (#6944); it no longer builds from
`crates/openhuman-cli`. From a checkout of that repository:

```bash
./profile/scripts/rss-bench.sh
```

## Layout

| Path | What it does |
| --- | --- |
| [`Cargo.toml`](Cargo.toml) | Standalone package with an empty `[workspace]` table. |
| [`src/main.rs`](src/main.rs) | Argument parsing, the sampling loop, process classification, report building, markdown rendering and stack parsing. |

## Gotchas

- `classify_process` still names CEF roles ("CEF renderer", "CEF GPU", "CEF
  utility", "CEF other") by matching `--type=` flags and words such as
  `renderer` in the command line. The app now runs on Wry, whose webview
  helper processes do not use those names, so descendants mostly land in
  "Other child process". On macOS, WKWebView's WebContent and GPU processes
  are started by the system rather than as children of the app, so they fall
  outside the host's process tree and are not counted at all.
- The default output path is relative to the working directory. Through
  `pnpm profile:tauri` that is the repository root.

## Tests

[`src/main_tests.rs`](src/main_tests.rs) covers argument parsing, process classification and tree grouping, report building, and stack parsing.
This crate is outside both the root workspace and the app crate, so run it on
its own:

```bash
cargo test --manifest-path crates/openhuman-app/profiling/Cargo.toml
```

## Further reading

- [`docs/gitbooks/en/developing/performance.md`](../../../docs/gitbooks/en/developing/performance.md): performance.
- [`docs/gitbooks/en/developing/architecture/tauri-shell.md`](../../../docs/gitbooks/en/developing/architecture/tauri-shell.md): the Tauri shell.
- [`crates/openhuman-app/README.md`](../README.md): the openhuman-app crate README.
