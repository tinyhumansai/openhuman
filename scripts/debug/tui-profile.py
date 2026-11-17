#!/usr/bin/env python3
"""Measure synthetic input in an isolated POSIX PTY with no operator credentials."""
import argparse
import codecs
import errno
import fcntl
import hashlib
import json
import os
import platform
import re
import selectors
import struct
import subprocess
import termios
import time
import tempfile
import unicodedata
from pathlib import Path

CSI = re.compile(rb"\x1b\[[0-?]*[ -/]*[@-~]")


class Screen:
    """Track the cursor/erase subset emitted by crossterm's differential renderer.

    Searching stripped bytes alone fails for individual key events: unchanged
    cells are omitted, and erasing a placeholder can interrupt the marker.
    """

    def __init__(self, columns, rows):
        self.columns, self.rows = columns, rows
        self.x = self.y = 0
        self.cells = {}
        self.pending = b""
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")

    def contains(self, marker):
        marker = marker.decode("ascii")
        return any(marker in "".join(self.cells.get((x,y), " ")
                    for x in range(self.columns)) for y in range(self.rows))

    def feed(self, raw):
        self.pending += raw
        while self.pending:
            if self.pending.startswith(b"\x1b["):
                match = CSI.match(self.pending)
                if not match:
                    return
                sequence = match.group()
                self.pending = self.pending[len(sequence):]
                params = sequence[2:-1].decode("ascii")
                if params.startswith(("?",">","<")):
                    continue
                values = [int(part) if part.isdigit() else 0 for part in params.split(";")]
                first = values[0] or 1
                command = sequence[-1:]
                if command in (b"H",b"f"):
                    self.y = first - 1
                    self.x = (values[1] if len(values)>1 and values[1] else 1) - 1
                elif command == b"G": self.x = first - 1
                elif command == b"d": self.y = first - 1
                elif command == b"A": self.y = max(0,self.y-first)
                elif command == b"B": self.y += first
                elif command == b"C": self.x += first
                elif command == b"D": self.x = max(0,self.x-first)
                elif command == b"J":
                    mode = values[0]
                    for position in list(self.cells):
                        if mode in (2,3) or (mode == 0 and (position[1],position[0]) >= (self.y,self.x)) \
                                or (mode == 1 and (position[1],position[0]) <= (self.y,self.x)):
                            del self.cells[position]
                elif command == b"K":
                    mode = values[0]
                    for x in range(self.columns):
                        if mode == 2 or (mode == 0 and x >= self.x) or (mode == 1 and x <= self.x):
                            self.cells.pop((x,self.y),None)
                continue
            if self.pending.startswith(b"\x1b]"):
                endings = [index for index in (self.pending.find(b"\x07"),self.pending.find(b"\x1b\\"))
                           if index >= 0]
                if not endings:
                    return
                end = min(endings)
                self.pending = self.pending[end+(1 if self.pending[end:end+1] == b"\x07" else 2):]
                continue
            if self.pending.startswith(b"\x1b"):
                if len(self.pending)<2:
                    return
                self.pending = self.pending[2:]
                continue
            end = self.pending.find(b"\x1b")
            end = end if end >= 0 else len(self.pending)
            plain = self.decoder.decode(self.pending[:end])
            self.pending = self.pending[end:]
            for char in plain:
                if char == "\r": self.x = 0
                elif char == "\n": self.y = min(self.rows-1,self.y+1)
                elif char == "\b": self.x = max(0,self.x-1)
                elif ord(char) >= 32 and not unicodedata.combining(char):
                    if self.x >= self.columns:
                        self.x = 0
                        self.y = min(self.rows-1,self.y+1)
                    self.cells[self.x,self.y] = char
                    self.x += 2 if unicodedata.east_asian_width(char) in ("W","F") else 1


def percentile(values, fraction):
    ordered = sorted(values)
    return round(ordered[min(int(len(ordered) * fraction), len(ordered) - 1)], 3)


def process_stats(pid):
    values = subprocess.check_output(
        ["ps", "-p", str(pid), "-o", "time=,rss="], text=True
    ).split()
    clock, rss = values[-2:]
    seconds = sum(float(part) * (60 ** index) for index, part in enumerate(reversed(clock.split(":"))))
    return seconds, int(rss)


def profile(binary, columns, rows, samples, idle_seconds, mode, clear_gap, input_mode):
    import pty

    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))

    def child_terminal():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

    scratch_root = Path(__file__).resolve().parents[2] / "target/debug-logs/tui-profile"
    scratch_root.mkdir(parents=True, exist_ok=True)
    scratch = tempfile.TemporaryDirectory(prefix="isolated-", dir=scratch_root)
    # An allowlist avoids inheriting API keys, session tokens, debug delays, or
    # telemetry configuration. CWD and workspace are fresh and contain no .env.
    environment = {key: os.environ[key] for key in ("PATH", "LANG", "LC_ALL", "TMPDIR")
                   if key in os.environ}
    dotenv = Path(scratch.name) / "empty.env"
    dotenv.touch()
    workspace = Path(scratch.name) / "workspace"
    workspace.mkdir()
    action_dir = Path(scratch.name) / "actions"
    action_dir.mkdir()
    environment.update({"TERM": "xterm-256color", "OPENHUMAN_WORKSPACE":str(workspace),
                        "OPENHUMAN_ACTION_DIR":str(action_dir),
                        "OPENHUMAN_DOTENV_PATH":str(dotenv)})
    if mode == "live":
        # Demo suppresses telemetry even when checking an older binary that
        # has not yet gained --no-telemetry. --help returns before demo starts.
        help_output = subprocess.run([str(binary), "--demo", "--help"], env=environment,
                                     cwd=scratch.name, capture_output=True, timeout=10, check=True)
        if b"--no-telemetry" not in help_output.stdout:
            os.close(master)
            os.close(slave)
            scratch.cleanup()
            raise RuntimeError("--mode live requires a binary advertising --no-telemetry; rebuild first")
    started = time.perf_counter()
    process = subprocess.Popen(
        [str(binary), "--demo"] if mode == "demo" else [str(binary), "--no-telemetry", "--new"],
        stdin=slave, stdout=slave, stderr=slave,
        preexec_fn=child_terminal, close_fds=True,
        env=environment, cwd=scratch.name,
    )
    os.close(slave)
    os.set_blocking(master, False)
    selector = selectors.DefaultSelector()
    selector.register(master, selectors.EVENT_READ)
    query_tail = b""
    cursor_queries = 0
    screen = Screen(columns, rows)

    def collect(duration, marker=None):
        nonlocal query_tail, cursor_queries
        data = bytearray()
        deadline = time.perf_counter() + duration
        while time.perf_counter() < deadline:
            for _key, _events in selector.select(max(0, deadline - time.perf_counter())):
                try:
                    chunk = os.read(master, 65536)
                except OSError as error:
                    if error.errno == errno.EIO:
                        return bytes(data)
                    raise
                if not chunk:
                    return bytes(data)
                data.extend(chunk)
                screen.feed(chunk)
                # A PTY is a byte pipe, not a terminal emulator. ratatui asks
                # for the cursor position while constructing its Terminal;
                # crossterm waits for a DSR reply before it can paint a frame.
                pending = query_tail + chunk
                cursor_queries += pending.count(b"\x1b[6n")
                for _ in range(pending.count(b"\x1b[6n")):
                    os.write(master, b"\x1b[1;1R")
                query_tail = pending[-3:]
                if marker is not None and screen.contains(marker):
                    return bytes(data)
        if marker is not None:
            raise RuntimeError("TUI did not acknowledge synthetic input before deadline; "
                               f"child exit={process.poll()}, synthetic output tail={bytes(data[-512:])!r}")
        return bytes(data)

    try:
        collect(30, b"Offline preview" if mode == "demo" else b"Signed out")
        startup_ms = (time.perf_counter() - started) * 1000
        collect(0.2)  # Drain initial paint before measuring idle output.
        before_cpu, before_rss = process_stats(process.pid)
        idle_output = collect(idle_seconds)
        after_cpu, after_rss = process_stats(process.pid)
        latency = []
        output_bytes = 0
        for sample in range(samples):
            os.write(master, b"\x03")
            collect(clear_gap)
            # Different full text each time makes a ratatui diff observable
            # even if clearing and pasting coalesce into a single frame.
            suffix = str(sample).encode("ascii")
            marker = bytes([65 + sample % 26]) * min(20, columns - 5 - len(suffix)) + suffix
            began = time.perf_counter_ns()
            os.write(master, b"\x1b[200~" + marker + b"\x1b[201~"
                     if input_mode == "paste" else marker)
            response = collect(2, marker)
            latency.append((time.perf_counter_ns() - began) / 1_000_000)
            output_bytes += len(response)
        os.write(master, b"\x04")
        collect(0.5)  # Drain teardown escapes so the PTY cannot block a final write.
        process.wait(timeout=3)
        return {
            "schema": 1, "mode": ("offline demo in POSIX PTY; no core or model calls"
                if mode == "demo" else "live event loop in POSIX PTY; fresh workspace; no login or model calls"),
            "terminal": {"columns": columns, "rows": rows},
            "startup_first_frame_ms": round(startup_ms, 3),
            "idle": {"seconds": idle_seconds, "terminal_output_bytes": len(idle_output),
                     "cpu_ms": round((after_cpu - before_cpu) * 1000, 3),
                     "cpu_counter_resolution_ms": 10 if platform.system() == "Darwin" else 1000,
                     "rss_kib": max(before_rss, after_rss)},
            "input_to_paint": {"samples": samples, "p50_ms": percentile(latency, 0.5),
                               "p95_ms": percentile(latency, 0.95),
                               "input_mode":input_mode,
                               "clear_to_input_gap_ms":clear_gap * 1000,
                               "max_ms": round(max(latency), 3), "output_bytes": output_bytes},
            "exit_code": process.returncode,
            "terminal_emulation": {"cursor_position_replies":cursor_queries,
                                   "marker_detection":"screen cells after cursor and erase processing"},
        }
    finally:
        selector.close()
        os.close(master)
        try:
            if process.poll() is None:
                # Closing this isolated controlling PTY delivers SIGHUP first.
                # Signals below always target this exact child, never a group
                # containing another developer's shell or terminal session.
                try:
                    process.wait(timeout=0.5)
                except subprocess.TimeoutExpired:
                    process.terminate()
                    try:
                        process.wait(timeout=2)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
        finally:
            scratch.cleanup()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/openhuman-tui"))
    parser.add_argument("--columns", type=int, default=80)
    parser.add_argument("--rows", type=int, default=24)
    parser.add_argument("--samples", type=int, default=40)
    parser.add_argument("--idle-seconds", type=float, default=2)
    parser.add_argument("--mode", choices=("demo","live"), default="demo")
    parser.add_argument("--clear-gap-ms", type=float, default=10)
    parser.add_argument("--input-mode", choices=("paste","keys"), default="paste")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.samples < 1 or args.idle_seconds <= 0 or args.columns < 24 or args.rows < 8 or args.clear_gap_ms < 0:
        parser.error("Use at least one sample, positive idle time, and a terminal >=24x8")
    binary = args.binary.resolve(strict=True)
    binary_before = binary.stat()
    with binary.open("rb") as binary_file:
        binary_sha256 = hashlib.file_digest(binary_file, "sha256").hexdigest()
    result = profile(binary, args.columns, args.rows, args.samples, args.idle_seconds,
                     args.mode, args.clear_gap_ms / 1000, args.input_mode)
    result["environment"] = {"platform": platform.platform(), "machine": platform.machine(),
                             "python": platform.python_version()}
    result["binary_sha256"] = binary_sha256
    binary_after = binary.stat()
    result["binary_changed_during_run"] = (binary_before.st_ino,binary_before.st_mtime_ns,binary_before.st_size) \
        != (binary_after.st_ino,binary_after.st_mtime_ns,binary_after.st_size)
    text = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(text)
    print(text, end="")


if __name__ == "__main__":
    main()
