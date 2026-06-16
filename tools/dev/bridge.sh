#!/usr/bin/env bash
# Dev-bridge curl cookbook for `life` (see DEV_BRIDGE.md).
#
# Launch the app first:   cargo run --features dev &
# Then load the recipes:  source tools/dev/bridge.sh
# ...and call them:        bstatus ; bshot s.png ; brun 6 6000 25 ; bstatus
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
bstatus()  { J animata/status; }                       # full stats snapshot + controls
bhist()    { J animata/histogram; }                    # per-layer / appendage / segment / hidden spreads
binspect() { J animata/inspect "{\"x\":${1:-4400},\"y\":${2:-3040}}"; }  # nearest creature to a world point
binspectid() { J animata/inspect "{\"id\":$1}"; }      # a specific creature by id
bping()    { curl -s -m 2 "http://${BRIDGE_HOST}:${BRIDGE_PORT}" -d '{"jsonrpc":"2.0","id":1,"method":"animata/status"}' >/dev/null && echo up || echo down; }

# one numeric field out of status (needs jq): bget frac_underground
bget()     { J animata/status | jq -r ".result.$1"; }

# ── controls ─────────────────────────────────────────────────────────────────
bpause()   { J animata/set_pause '{"paused":true}'; }
bresume()  { J animata/set_pause '{"paused":false}'; }
bspeed()   { J animata/set_speed "{\"steps\":${1:-8}}"; }
bstep()    { J animata/step "{\"n\":${1:-1}}"; }        # advance n steps (works while paused)
breset()   { J animata/reset "{\"seed\":${1:-6}}"; }
bview()    { J animata/set_view "{\"scale\":${1:-9},\"cx\":${2:-4400},\"cy\":${3:-3040}}"; }
bcolor()   { J animata/set_color "{\"mode\":\"${1:-species}\"}"; }   # diet|lineage|species
bselect()  { J animata/select "{\"x\":${1:-4400},\"y\":${2:-3040}}"; }
bparam()   { J animata/set_param "{\"name\":\"$1\",\"value\":$2}"; } # food_per_step|predator_gain|mutation_rate
bsave()    { J animata/save "{\"path\":\"${1:-animata_save.txt}\"}"; }
bload()    { J animata/load "{\"path\":\"${1:-animata_save.txt}\"}"; }
bshot()    { J animata/screenshot "{\"path\":\"${1:-shot.png}\"}"; } # PNG to repo dir; then Read it

# ── combos ───────────────────────────────────────────────────────────────────
# brun <seed> <steps> <wait_s> — reset, run at speed, wait, then print status.
# Use this to evolve a fresh world and read the emergent outcome in one call.
brun() {
  local seed="${1:-6}" steps="${2:-6000}" wait_s="${3:-25}"
  breset "$seed" >/dev/null; bresume >/dev/null; bspeed 12 >/dev/null
  echo "running seed $seed for ~${wait_s}s..."; sleep "$wait_s"; bstatus
}

# bzoomshot <scale> <cx> <cy> <path> — frame a spot and capture it (pause first for a clean frame).
bzoomshot() { bpause >/dev/null; bview "${1:-9}" "${2:-4400}" "${3:-3040}" >/dev/null; bshot "${4:-shot.png}"; }
