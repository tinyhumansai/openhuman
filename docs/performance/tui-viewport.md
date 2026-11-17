# TUI viewport measurements

Measured 2026-10-10 on an Apple M2 Pro, x86_64 macOS process, using the
same warm Cargo development profile before and after the viewport change.
These are actual elapsed measurements, not timing assertions in tests.

Reproduce from the checkout root:

```sh
scripts/ci-cancel-aware.sh cargo build -p openhuman-tui
target/debug/openhuman-tui --bench
```

The command creates deterministic synthetic transcripts, uses the production
viewport and renderer, and prints JSON. It neither boots the core nor reads
operator conversations or credentials. Each measurement has 20 warmups and
200 timed samples. The initial comparison used 500, 10,000, or 40,000 settled
entries, each occupying two rows at both tested widths. The JSON calls these
`entries` and separately reports `settled_rows` (1,000, 20,000, and 80,000).
The viewport workload uses width minus four and height minus nine, matching
the empty-composer chat geometry. Scroll samples advance three rows at a time.
Streaming samples append the same 15-byte delta and redraw the growing tail;
they include reducer processing. Full frames use ratatui's TestBackend.

Raw runs are stored locally in
`target/debug-logs/tui-performance/{before,after}.json`.
Times below are microseconds, shown as p50 / p95.

| Terminal | Entries | Viewport idle before | Viewport idle after |     Scroll before |    Scroll after |
| -------- | ------: | -------------------: | ------------------: | ----------------: | --------------: |
| 80x24    |     500 |      12.209 / 27.291 |     11.750 / 12.208 |   15.250 / 18.208 | 15.208 / 32.708 |
| 80x24    |  10,000 |    210.334 / 235.334 |       3.917 / 4.041 | 215.083 / 234.458 |  9.917 / 13.417 |
| 80x24    |  40,000 |    838.375 / 859.916 |       4.250 / 4.375 | 843.042 / 964.208 | 10.583 / 14.333 |
| 120x40   |     500 |      11.875 / 12.000 |       5.125 / 5.250 |   14.166 / 16.792 | 11.042 / 15.708 |
| 120x40   |  10,000 |    210.584 / 224.125 |       5.334 / 5.500 | 216.250 / 248.417 | 11.208 / 14.750 |
| 120x40   |  40,000 |    840.500 / 933.166 |       5.375 / 5.667 | 842.667 / 891.667 | 11.292 / 14.917 |

| Terminal | Entries |   Full frame before |    Full frame after | Streaming tail before | Streaming tail after |
| -------- | ------: | ------------------: | ------------------: | --------------------: | -------------------: |
| 80x24    |     500 |   562.709 / 758.958 |   555.042 / 751.167 |     111.000 / 157.542 |     83.833 / 142.375 |
| 80x24    |  10,000 |   758.750 / 868.000 |   590.833 / 707.667 |     290.916 / 350.459 |     87.625 / 150.084 |
| 80x24    |  40,000 | 1384.750 / 1432.416 |   566.125 / 632.500 |    926.792 / 1062.959 |     86.667 / 148.417 |
| 120x40   |     500 | 1074.833 / 1095.875 | 1112.417 / 1145.500 |      88.917 / 145.000 |     85.125 / 144.208 |
| 120x40   |  10,000 | 1277.250 / 1330.959 | 1117.250 / 1175.833 |     290.417 / 345.416 |     85.417 / 145.042 |
| 120x40   |  40,000 | 1936.625 / 2056.208 | 1124.250 / 1162.542 |    923.125 / 1009.958 |     88.000 / 146.042 |

An expanded tool entry originating from 40,000 lines (approximately 1 MB)
measured 0.958 / 1.000 before and 1.083 / 1.250 after in warm viewport
rendering. This exercises the existing bounded activity projection and
20-line expanded display. Ingesting the original output is outside that
measurement; these numbers do not claim 1 MB of text can be parsed in 1 us.

The runner also includes 250, 5,000, and 20,000 entries to cover exactly 500,
10,000, and 40,000 settled rows. A supplemental optimized run, saved in
`target/debug-logs/tui-performance/after-exact-rows.json`, measured:

| Terminal | Settled rows | Viewport idle |          Scroll |          Full frame |   Streaming tail |
| -------- | -----------: | ------------: | --------------: | ------------------: | ---------------: |
| 80x24    |          500 | 4.542 / 5.375 | 11.125 / 15.958 |   583.541 / 761.209 | 85.833 / 145.417 |
| 80x24    |       10,000 | 4.000 / 4.125 |  9.834 / 13.417 |   556.042 / 613.625 | 81.625 / 141.542 |
| 80x24    |       40,000 | 4.000 / 4.167 |  9.834 / 13.375 |   547.333 / 561.291 | 82.042 / 140.000 |
| 120x40   |          500 | 5.208 / 5.417 |  9.709 / 13.875 | 1070.042 / 1110.125 | 81.291 / 136.583 |
| 120x40   |       10,000 | 5.208 / 5.334 | 10.708 / 14.333 | 1068.292 / 1097.167 | 80.625 / 136.708 |
| 120x40   |       40,000 | 5.209 / 5.375 | 10.625 / 14.125 | 1077.125 / 1176.166 | 80.667 / 140.000 |

## Finding and change

Although wrapped text was cached, every frame still visited every entry to
check its revision and rebuild cumulative heights, then visited every cached
block again to evict rows. At 40,000 entries this cost about 0.84 ms even when
the conversation had not changed. Separately timed viewport and full-frame
measurements show that history scan accounting for most of the frame cost
increase between 500 and 40,000 entries.

The reducer now supplies a bounded 128-change journal. Unchanged snapshots
need no history scan; point changes identify exact blocks, including older
tool entries. A dynamic Fenwick tree updates heights and locates the visible
entry in logarithmic time. Eviction visits only resident formatted blocks,
bounded to a window of at most 193 entries. Rendering skips the offscreen
prefix inside a large block. At 40,000 entries the measured idle viewport
p50 improves about 197 times at 80x24; the full frame improves 2.45 times.

The composer is anchored to the bottom of the chat area with its status line
below it. Its minimum height reserves separate editor and control rows even
at 24x8. Resize reflows the editor and preserves a scrolled transcript's top
entry and relative wrapped line; following the latest message stays at the
bottom. Too-small terminals clear stale hit geometry.

## Actual terminal measurements

The separate POSIX PTY runner exercises the real binary and crossterm input:

```sh
python3 scripts/debug/tui-profile.py --columns 80 --rows 24 --samples 40 \
  --output target/debug-logs/tui-performance/pty-after-80x24.json
python3 scripts/debug/tui-profile.py --columns 120 --rows 40 --samples 40 \
  --output target/debug-logs/tui-performance/pty-after-120x40.json
```

It starts an isolated `--demo` child, uses fresh workspace and action directories
plus an explicit empty dotenv file under `target/debug-logs`, and allowlists inherited
environment variables so keys, sessions, telemetry settings, and restart
delays cannot leak into the run. It answers cursor-position queries if a
terminal backend emits them (the measured binary emitted none). Teardown
drains escape output before waiting for the child to exit. Synthetic input
uses bracketed paste, with changing text so ratatui's differential output
still contains an observable marker. A small cursor/erase screen tracker
recognizes the completed marker even when individual key frames omit unchanged
cells. Each paste follows a clear operation
by 10 ms. Both measured processes exited normally with code zero.

| Terminal | Startup to footer paint | Idle output over 2 s | Idle CPU |        RSS |  Input p50 / p95 | Input max |
| -------- | ----------------------: | -------------------: | -------: | ---------: | ---------------: | --------: |
| 80x24    |               62.178 ms |              0 bytes |     0 ms | 16,208 KiB | 3.726 / 4.354 ms |  4.764 ms |
| 120x40   |               60.245 ms |              0 bytes |     0 ms | 16,480 KiB | 3.856 / 6.452 ms |  6.495 ms |

A supplemental 80x24 run with `--input-mode keys --samples 20` sent 21 or
22 individual key events per burst. It measured 13.043 / 13.568 ms p50 / p95
and 19,300 total output bytes, compared with 4,720 bytes for 40 paste samples.
This reflects the demo loop painting after each key; paste is one event.

CPU is measured with `ps` and has 10 ms resolution on macOS: a zero reading
does not prove zero instructions executed. RSS is a process snapshot rather
than a peak allocation counter. Input measurements include event parsing,
rendering, terminal writes, and the profiler's scheduling. Startup has one
sample per terminal and includes process spawn and library initialization.
These are optimized-binary observations with no PTY baseline; no terminal
before/after improvement is claimed. Demo mode boots neither core nor model
and does not measure the live loop's 16 ms frame coalescing.

The screen parser has four deterministic tests for differential paints,
placeholder erasure, split cursor/OSC sequences, styles, and split UTF-8 with
wide and combining characters. Run them with:

```sh
python3 scripts/debug/tui-profile_tests.py
```

### Live event loop

Four further runs use `--mode live`, which starts the real in-process core and
event loop with `--new --no-telemetry`. Workspace, action directory, credential
scope, and dotenv are fresh; no sign-in or model request is initiated. Each
case has 40 input samples and a two-second idle window. The final binary uses
a 16 ms minimum frame interval while dirty and draws no idle frames. All four
saved reports have the same SHA-256
`97d86f1428e49e5e534654278c04d4c79939fb498e8cb4c072814d496ae39850`,
report no binary change during the run, and exited normally with code zero.

```sh
python3 scripts/debug/tui-profile.py --mode live --input-mode paste \
  --columns 80 --rows 24 --samples 40 \
  --output target/debug-logs/tui-performance/pty-live-paste-80x24.json
```

Repeat with `--input-mode keys` and/or `--columns 120 --rows 40` for the other
cases. Paste supplies one event; keys supplies 21 or 22 individual events.
The 80x24 paste case was repeated after a concurrent offline benchmark ended;
the table uses that quiet repeat.

| Terminal | Input |    Startup | Idle output | Idle CPU |        RSS |    Input p50 / p95 | Input max | Total input output |
| -------- | ----- | ---------: | ----------: | -------: | ---------: | -----------------: | --------: | -----------------: |
| 80x24    | Paste | 410.989 ms |     0 bytes |     0 ms | 78,976 KiB |   7.509 / 8.688 ms |  8.988 ms |        2,716 bytes |
| 120x40   | Paste | 428.299 ms |     0 bytes |    10 ms | 80,176 KiB | 11.620 / 15.032 ms | 15.603 ms |        2,756 bytes |
| 80x24    | Keys  | 400.491 ms |     0 bytes |     0 ms | 79,712 KiB | 10.323 / 13.254 ms | 13.614 ms |        2,716 bytes |
| 120x40   | Keys  | 399.581 ms |     0 bytes |     0 ms | 79,552 KiB | 11.241 / 18.511 ms | 33.412 ms |        2,803 bytes |

The live key bursts emit about as many bytes as single paste events because
the dirty-frame interval coalesces individual key paints. The interval is
not a hard input latency guarantee: parser delivery, draw time, and operating
system scheduling remain outside that interval, and the 120x40 key run shows
a 33.412 ms outlier. These measurements include actual terminal I/O and core
startup; the viewport/TestBackend tables measure separate renderer work with
synthetic settled history. No PTY baseline or network/auth responsiveness
improvement is claimed. Credential-free startup is not a benchmark of restoring
an existing user's account.

The final offline binary was also checked for consistency: at 80x24 and 40,000
settled rows (20,000 entries), viewport idle measured 4.208 / 5.459 us and full
frame idle measured 575.084 / 622.625 us; at 80,000 settled rows (40,000 entries),
the values were 4.291 / 4.584 us and 576.000 / 602.792 us. These final observations
support the earlier scaling result; they do not replace the paired before/after
measurements.

## Verification and limits

Behavior tests compare indexed output to full reference wrapping across
widths, old tool updates, expansion, journal expiry, clear, and replacement.
They also verify idle recomputation is zero, an append or point edit
recomputes one block, the resident window is bounded, cloned transcripts
cannot reuse another branch's snapshot, and resize preserves the viewed
entry. Composer tests resize through 80x24, 120x40, 40x12, and 24x8 and
verify cursor and hit targets remain inside the current editor geometry.
There are no flaky elapsed-time assertions.

These results are development-profile measurements, with no login, model
inference, or release-performance claim. Viewport/TestBackend measurements
exclude terminal I/O; the PTY measurements include it. Concurrent development
on the same machine adds noise; the 500-entry scrolling p95 and 120x40 small
full-frame case show small regressions in this run. Cold initialization,
width changes, transcript replacement, and expired journals still rebuild
history and are not covered by the warm figures. Metadata scales with entry
count, while formatted row retention is bounded by entry count rather than
bytes. A very large assistant message remains one block and streaming it
rewraps that block; increasing its length can still increase streaming cost.
