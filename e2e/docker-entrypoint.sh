#!/usr/bin/env bash
#
# Entrypoint for the Linux E2E Docker container.
# Starts Xvfb (virtual display) and dbus before running the test command.
#
set -euo pipefail

# Start virtual framebuffer (required for webkit2gtk rendering)
export DISPLAY=:99
Xvfb :99 -screen 0 1280x1024x24 &
XVFB_PID=$!

# Clean up Xvfb on exit so the container stops promptly
cleanup() {
  if [ -n "${XVFB_PID:-}" ] && kill -0 "$XVFB_PID" 2>/dev/null; then
    kill "$XVFB_PID" 2>/dev/null || true
    wait "$XVFB_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

# Verify Xvfb started — retry a few times to cover fast exits
for i in 1 2 3 4 5; do
  if kill -0 "$XVFB_PID" 2>/dev/null; then
    break
  fi
  if [ "$i" -eq 5 ]; then
    echo "ERROR: Xvfb (pid $XVFB_PID) failed to start." >&2
    exit 1
  fi
  sleep 0.5
done

# Start dbus session (required by webkit2gtk for IPC)
eval "$(dbus-launch --sh-syntax)"

# Ensure XDG dirs exist for deep-link registration
mkdir -p ~/.local/share/applications

# Export backtrace for debugging
export RUST_BACKTRACE=1

echo "Xvfb started on $DISPLAY (pid $XVFB_PID)"
echo "D-Bus session: $DBUS_SESSION_BUS_ADDRESS"

# Run the provided command (default: yarn workspace openhuman-app test:e2e:all)
exec "$@"
