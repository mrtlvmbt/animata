#!/usr/bin/env bash
# Dev-bridge curl cookbook for the voxel viewer (see DEV_BRIDGE.md).
#
# Launch the app first:   cargo run --features dev &
# Then load the recipes:  source tools/dev/bridge.sh
# ...and call them:        bstatus ; bview 70 47 170 ; breseed 5 ; bshot s.png
#
# Keep this file as the single place curl recipes live — add a function here the
# moment a useful one-liner proves itself, so it can be pulled and run instantly.

BRIDGE_PORT="${BRIDGE_PORT:-8127}"
BRIDGE_HOST="${BRIDGE_HOST:-127.0.0.1}"

# Pretty-printer: jq if present, else python, else raw.
_bpp() { if command -v jq >/dev/null; then jq .; elif command -v python3 >/dev/null; then python3 -m json.tool; else cat; fi; }

# J <method> [json-params]  — raw JSON-RPC call, pretty-printed.
J() {
  curl -s -m 5 "http://${BRIDGE_HOST}:${BRIDGE_PORT}" \
    -H 'Content-Type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":${2:-null}}" | _bpp
}

# ── reads ────────────────────────────────────────────────────────────────────
bstatus()  { J animata/status; }                       # fps + frame_ms + camera + map
bping()    { curl -s -m 2 "http://${BRIDGE_HOST}:${BRIDGE_PORT}" -d '{"jsonrpc":"2.0","id":1,"method":"animata/status"}' >/dev/null && echo up || echo down; }
# one numeric field out of status (needs jq):  bget fps | bget frame_ms | bget seed
bget()     { J animata/status | jq -r ".result.$1"; }

# ── controls ─────────────────────────────────────────────────────────────────
# bview <cx> <cz> [zoom] [yaw]  — move/zoom/rotate the iso camera
bview()    { J animata/set_view "{\"cx\":${1:-69},\"cz\":${2:-47},\"zoom\":${3:-170},\"yaw\":${4:-0}}"; }
bzoom()    { J animata/set_view "{\"zoom\":${1:-170}}"; }
breseed()  { J animata/reseed "{\"seed\":${1:-1}}"; }  # regenerate the world
bshot()    { J animata/screenshot "{\"path\":\"${1:-shot.png}\"}"; }  # PNG from offscreen target (works unfocused); then Read it

# ── combos ───────────────────────────────────────────────────────────────────
# bframe <seed> <cx> <cz> <zoom> <path> — reseed, frame a spot, capture it.
bframe() {
  breseed "${1:-1}" >/dev/null
  bview "${2:-69}" "${3:-47}" "${4:-170}" >/dev/null
  bshot "${5:-shot.png}"
}
