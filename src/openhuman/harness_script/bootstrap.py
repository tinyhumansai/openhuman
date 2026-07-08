"""Child-side protocol loop for the harness-script runner (Phase 1).

Speaks the newline-delimited JSON protocol defined in `protocol.rs`:

    1. emit `{"type": "ready", "protocol_version": N}` on startup
    2. read one `{"type": "init", ...}` line from stdin
    3. exec the supplied script, exposing `inputs` and `set_result(value)`
    4. emit exactly one terminal `{"type": "result"|"error", ...}` line

Only stdout carries protocol frames; everything diagnostic goes to stderr so
the Rust runner can treat the two streams separately.

PHASE 4: the script is exec'd WITHOUT a sandbox. Static AST validation, import
and call restrictions, and approval gating are intentionally out of Phase 1
scope (see the design spec). This file must not gain any real capability until
that sandbox exists.
"""

import json
import sys
import traceback

PROTOCOL_VERSION = 1

# Bind the protocol channel to the real stdout at import time. The script's own
# stdout is redirected to stderr during exec (see `_run`), so `print()` from
# user code can never inject bytes into the protocol stream.
_PROTOCOL_OUT = sys.stdout


def _emit(message):
    """Write a single protocol frame to the protocol channel and flush."""
    _PROTOCOL_OUT.write(json.dumps(message) + "\n")
    _PROTOCOL_OUT.flush()


def _log(text):
    """Diagnostic output on stderr; never parsed as protocol."""
    sys.stderr.write(text + "\n")
    sys.stderr.flush()


def _run(script, inputs):
    """Execute the script, returning the value passed to set_result()."""
    captured = {"value": None}

    def set_result(value):
        captured["value"] = value

    namespace = {
        "__name__": "__harness_script__",
        "inputs": inputs,
        "set_result": set_result,
    }
    # Redirect the script's stdout to stderr so any print()/stdout writes from
    # user code land on the log channel instead of corrupting protocol frames.
    saved_stdout = sys.stdout
    sys.stdout = sys.stderr
    try:
        # PHASE 4: unsandboxed exec — see module docstring.
        exec(script, namespace)  # noqa: S102
    finally:
        sys.stdout = saved_stdout
    return captured["value"]


def main():
    _emit({"type": "ready", "protocol_version": PROTOCOL_VERSION})

    line = sys.stdin.readline()
    if not line:
        _log("harness_script bootstrap: stdin closed before init")
        return 1

    try:
        init = json.loads(line)
    except json.JSONDecodeError as exc:
        _emit(
            {
                "type": "error",
                "error_class": "ProtocolError",
                "message": f"init was not valid JSON: {exc}",
                "traceback": None,
            }
        )
        return 1

    if init.get("type") != "init":
        _emit(
            {
                "type": "error",
                "error_class": "ProtocolError",
                "message": f"expected init frame, got type={init.get('type')!r}",
                "traceback": None,
            }
        )
        return 1

    script = init.get("script", "")
    inputs = init.get("inputs")

    try:
        output = _run(script, inputs)
        result_frame = {"type": "result", "output": output}
        # Force serialization inside the guarded block so a non-serializable
        # set_result(...) value is reported as a ScriptError rather than
        # crashing after the try and leaving the parent with no terminal frame.
        json.dumps(result_frame)
    except BaseException as exc:  # noqa: BLE001 — report every failure to the parent
        _emit(
            {
                "type": "error",
                "error_class": type(exc).__name__,
                "message": str(exc),
                "traceback": traceback.format_exc(),
            }
        )
        return 1

    _emit(result_frame)
    return 0


if __name__ == "__main__":
    sys.exit(main())
