# OpenHuman runtime-pool Python worker harness (issue #5106).
#
# A single long-lived `python` process that executes inline Python jobs for many
# skill runs, so the fleet pays one warm interpreter instead of one child per
# run.
#
# Protocol (newline-delimited JSON over an authenticated loopback socket,
# see runtime_pool/protocol.rs):
#   1. Print exactly one ready line: {"ready":true,"protocol":1,"lang":"python"}
#   2. For each request line {id,kind:"inline",code,cwd,timeout_ms} reply with
#      {id,ok,stdout,stderr,exit_code,timed_out,elapsed_ms,error}.
#
# Each job runs in this interpreter with stdout/stderr redirected into buffers so
# a job's prints never corrupt the protocol stream. Isolation is per-job globals
# plus the pool's recycle-after-N-jobs; CPython cannot safely kill a running
# thread, so the soft deadline is best-effort SIGALRM on Unix and otherwise the
# Rust side's hard deadline kills + respawns the worker.

import sys
import os
import json
import time
import tempfile
import traceback

PROTOCOL_VERSION = 1

# Production workers use one authenticated duplex socket for protocol traffic,
# leaving fd 0 at EOF and fd 1/2 entirely available to job capture. The stdio
# fallback keeps the harness convenient to launch by hand.
_PROTOCOL_TOKEN = os.environ.get("OPENHUMAN_RUNTIME_POOL_PROTOCOL_TOKEN")
_PROTOCOL_ADDR = os.environ.get("OPENHUMAN_RUNTIME_POOL_PROTOCOL_ADDR")
_PROTOCOL_SOCKET = None
if _PROTOCOL_ADDR:
    import socket

    _host, _port = _PROTOCOL_ADDR.rsplit(":", 1)
    _PROTOCOL_SOCKET = socket.create_connection((_host, int(_port)))
    _PROTO_IN = _PROTOCOL_SOCKET.makefile("r")
    _PROTO = _PROTOCOL_SOCKET.makefile("w", buffering=1)
else:
    # Private duplicates prevent per-job fd redirection from touching protocol.
    _PROTO_IN = os.fdopen(os.dup(0), "r", buffering=1)
    _PROTO = os.fdopen(os.dup(1), "w", buffering=1)

try:
    import signal

    _HAVE_ALARM = hasattr(signal, "SIGALRM") and hasattr(signal, "setitimer")
except Exception:  # pragma: no cover - platform without signal
    signal = None
    _HAVE_ALARM = False


class _JobTimeout(Exception):
    pass


def _run_job(job):
    code = job.get("code") or ""
    cwd = job.get("cwd")
    timeout_ms = job.get("timeout_ms")
    start = time.time()
    exit_code = 0
    timed_out = False
    extra_err = ""

    old_cwd = None
    if cwd:
        try:
            old_cwd = os.getcwd()
            os.chdir(cwd)
        except Exception as exc:
            # Match subprocess cwd semantics: if the requested action root
            # cannot be entered, user code must not run in this long-lived
            # worker's inherited directory.
            return {
                "id": job.get("id"),
                "ok": False,
                "stdout": "",
                "stderr": "",
                "exit_code": None,
                "timed_out": False,
                "elapsed_ms": int((time.time() - start) * 1000),
                "error": f"failed to set worker cwd: {exc!r}",
            }

    # Capture at the FILE-DESCRIPTOR level (not just `sys.stdout`) so
    # `os.write(1, ...)`, subprocesses, and native extensions are captured too —
    # otherwise they would leak onto the real stdout, which is the NDJSON
    # protocol channel. Temp files (vs pipes) avoid buffer-deadlock on large
    # output.
    in_f = tempfile.TemporaryFile(mode="w+b")
    out_f = tempfile.TemporaryFile(mode="w+b")
    err_f = tempfile.TemporaryFile(mode="w+b")
    saved_in = os.dup(0)
    saved_out = os.dup(1)
    saved_err = os.dup(2)
    os.dup2(in_f.fileno(), 0)
    os.dup2(out_f.fileno(), 1)
    os.dup2(err_f.fileno(), 2)

    armed = False
    if _HAVE_ALARM and timeout_ms and timeout_ms > 0:
        def _on_alarm(_signum, _frame):
            raise _JobTimeout()

        signal.signal(signal.SIGALRM, _on_alarm)
        signal.setitimer(signal.ITIMER_REAL, timeout_ms / 1000.0)
        armed = True

    try:
        # Fresh globals per job so top-level names don't leak between runs.
        g = {"__name__": "__main__", "__builtins__": __builtins__}
        exec(compile(code, "<inline>", "exec"), g, g)
    except _JobTimeout:
        timed_out = True
    except SystemExit as e:  # honour sys.exit(n)
        if e.code is None:
            exit_code = 0
        elif isinstance(e.code, int):
            exit_code = e.code
        else:
            exit_code = 1
            extra_err = str(e.code) + "\n"
    except BaseException:  # noqa: BLE001 - surface any job failure to the caller
        exit_code = 1
        extra_err = traceback.format_exc()
    finally:
        if armed:
            signal.setitimer(signal.ITIMER_REAL, 0)
        # Flush Python's buffers to the redirected fds, then restore the real
        # stdout/stderr before reading the captures. A flush failure is surfaced
        # in the job's stderr rather than silently discarded.
        try:
            sys.stdout.flush()
        except Exception as flush_err:  # noqa: BLE001
            extra_err += f"[harness] stdout flush failed: {flush_err!r}\n"
        try:
            sys.stderr.flush()
        except Exception as flush_err:  # noqa: BLE001
            extra_err += f"[harness] stderr flush failed: {flush_err!r}\n"
        os.dup2(saved_in, 0)
        os.dup2(saved_out, 1)
        os.dup2(saved_err, 2)
        os.close(saved_in)
        os.close(saved_out)
        os.close(saved_err)
        if old_cwd is not None:
            try:
                os.chdir(old_cwd)
            except Exception:
                pass

    in_f.close()
    out_f.seek(0)
    err_f.seek(0)
    stdout = out_f.read().decode("utf-8", "replace")
    stderr = err_f.read().decode("utf-8", "replace") + extra_err
    out_f.close()
    err_f.close()

    return {
        "id": job.get("id"),
        "ok": True,
        "stdout": stdout,
        "stderr": stderr,
        "exit_code": None if timed_out else exit_code,
        "timed_out": timed_out,
        "elapsed_ms": int((time.time() - start) * 1000),
        "error": None,
    }


def _reply(obj):
    _PROTO.write(json.dumps(obj) + "\n")
    _PROTO.flush()


def main():
    _reply({
        "ready": True,
        "protocol": PROTOCOL_VERSION,
        "lang": "python",
        "protocol_token": _PROTOCOL_TOKEN,
    })
    for line in _PROTO_IN:
        line = line.strip()
        if not line:
            continue
        try:
            job = json.loads(line)
        except Exception:
            continue  # ignore unparseable lines
        try:
            res = _run_job(job)
        except Exception as e:  # harness-level failure
            res = {
                "id": job.get("id") if isinstance(job, dict) else None,
                "ok": False,
                "stdout": "",
                "stderr": "",
                "exit_code": None,
                "timed_out": False,
                "elapsed_ms": 0,
                "error": repr(e),
            }
        _reply(res)


if __name__ == "__main__":
    main()
