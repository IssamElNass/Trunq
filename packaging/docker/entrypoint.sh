#!/bin/bash
set -euo pipefail
pids=()
cleanup() {
    trap - EXIT INT TERM
    if ((${#pids[@]})); then
        kill "${pids[@]}" 2>/dev/null || true
        wait "${pids[@]}" 2>/dev/null || true
    fi
}
trap cleanup EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
Xvfb "$DISPLAY" -screen 0 "${TRUNQ_SCREEN}" -nolisten tcp &
pids+=("$!")
for attempt in {1..100}; do
    if xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; then break; fi
    if ! kill -0 "${pids[0]}" 2>/dev/null; then exit 1; fi
    sleep 0.1
done
xdpyinfo -display "$DISPLAY" >/dev/null 2>&1
openbox --sm-disable &
pids+=("$!")
# VNC itself stays on loopback inside the container. Publish HTTP to host
# loopback as in compose.yaml; this desktop has no built-in authentication.
x11vnc -display "$DISPLAY" -forever -shared -nopw -localhost -rfbport 5900 -quiet &
pids+=("$!")
websockify --web=/usr/share/novnc 8080 127.0.0.1:5900 &
pids+=("$!")
dbus-run-session -- /usr/local/bin/trunq "$@" &
pids+=("$!")
# Stop the whole container if the client or a desktop service exits.
wait -n "${pids[@]}"
