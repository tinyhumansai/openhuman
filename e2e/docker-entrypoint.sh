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

# Wait for Xvfb to be ready
sleep 2

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
