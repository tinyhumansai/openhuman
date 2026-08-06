# Tauri resource profiler

Offline developer tool for measuring a locally running OpenHuman desktop app.
It is a standalone Cargo crate and is not linked into, registered with, or
shipped in the Tauri app.

## Capture

Start the current checkout:

    pnpm dev:app

Find the main host PID, excluding OpenHuman Helper processes:

    pgrep -fl '/OpenHuman$'

Then capture a representative workload:

    pnpm profile:tauri --pid <PID> --duration 15

Useful options:

    --interval-ms 250
    --out target/profile/my-scenario
    --no-stacks

The default output directory is target/profile/tauri-resources-<timestamp>.
Each run writes:

- resources.md: summary table for the Rust host, CEF process roles, and total
  desktop process tree.
- resources.json: raw time series plus mean and peak values.
- cpu-stacks.txt: macOS sample report for the host process. This is enabled by
  default on macOS and can be disabled with --no-stacks.

## Attribution boundary

The Rust core runs inside the Tauri host process. Operating-system CPU and RAM
metrics therefore cannot split the shell from openhuman_core, or assign heap
pages to individual Rust modules. The profiler reports that combined process
honestly as Tauri host + embedded Rust core.

CEF renderer, GPU, utility, and other helper processes are separate and are
reported independently. On macOS, the tool also parses the recursive stack
counts from cpu-stacks.txt and groups OpenHuman symbols by Rust domain, such as
openhuman_core::openhuman::agent or openhuman::core_process.

CPU percentages are percentages of one logical CPU and may exceed 100 when a
component uses multiple cores. RAM is resident memory reported by sysinfo.

For the smaller embedded-core-only Linux RSS/PSS benchmark, use the existing
root rss-bench harness:

    cargo build --release --features rss-bench --bin rss-bench
