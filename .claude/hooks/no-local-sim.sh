#!/usr/bin/env bash
# no-local-sim.sh — PreToolUse(Bash) guard: FORBID executing the animata simulation locally.
#
# WHY: the cloud is the AUTHORITATIVE gate (scripts/ci-report.sh for tests+goldens, scripts/sim-run.sh
# for heavy runs). Coders repeatedly run the sim on the dev machine (machine load + fabricated "green")
# — this hook makes that structurally impossible instead of a prompt-only contract. Applies to coders
# AND PM. COMPILE-ONLY stays allowed (--no-run / check / clippy / build compile but never execute).
#
# Blocks: `cargo test`/`cargo t` without --no-run, `cargo nextest run`, `cargo bench`, `test-bar.sh`,
# direct sim-binary runs (target/{debug,release}/*animata*|*sim*), `cargo run` of the sim.
# One-off conscious bypass (PM): `touch .claude/.sim-allow` then re-run — the file is consumed + logged
# in .claude/no-local-sim.log (mirrors the .done-allow / KIT_ALLOW_DIRTY override).
#
# stdin: JSON { tool_input: { command }, ... }.  Deny = stdout PreToolUse JSON (exit 0) — the settings.json
# wrapper `… && bash … || exit 0` normalizes the exit code, so blocking MUST go via stdout, not exit 2.
set -uo pipefail

PROJECT_DIR="${CLAUDE_PROJECT_DIR:-$(pwd)}"

CMD="$(python3 -c 'import sys,json
try:
    print(json.load(sys.stdin).get("tool_input",{}).get("command",""))
except Exception:
    print("")' 2>/dev/null)"
[ -z "$CMD" ] && exit 0

blocked=""
# Match ONLY in command position — after start-of-line or a real shell separator (; && || | & ( ),
# with optional leading whitespace and an optional `rtk`/`rtk proxy` wrapper. Plain whitespace and
# backticks are NOT separators, so a sim keyword quoted inside a commit message / echo / doc string
# (e.g. `cargo test` in this file's own commit body) is NOT a command and is left alone.
SEP='(^|&&|\|\||[;&|(])[[:space:]]*'
RTK='(rtk[[:space:]]+(proxy[[:space:]]+)?)?'
# Path prefix for a script/binary: optional bash|sh runner, optional `./`, path segments WITHOUT
# backtick/quote (so a backtick-wrapped mention can't masquerade as a path).
PFX='(bash[[:space:]]+|sh[[:space:]]+)?(\./)?([^[:space:]`'"'"'"]*/)?'

# cargo test / cargo t that EXECUTES. --no-run only compiles (allowed); --help is inert.
if printf '%s' "$CMD" | grep -qE "${SEP}${RTK}cargo[[:space:]]+(test|t)([[:space:]]|\$)" \
   && ! printf '%s' "$CMD" | grep -qE -- '--no-run|--help'; then
  blocked="cargo test executes the sim"
fi
printf '%s' "$CMD" | grep -qE "${SEP}${RTK}cargo[[:space:]]+nextest[[:space:]]+run" && blocked="cargo nextest run executes the sim"
printf '%s' "$CMD" | grep -qE "${SEP}${RTK}cargo[[:space:]]+bench"                   && blocked="cargo bench executes the sim"
printf '%s' "$CMD" | grep -qE "${SEP}${RTK}${PFX}test-bar\.sh"                       && blocked="test-bar.sh executes tests"
printf '%s' "$CMD" | grep -qE "${SEP}${RTK}${PFX}target/(debug|release)/[^[:space:]]*(animata|sim)" && blocked="running a sim binary directly"
if printf '%s' "$CMD" | grep -qE "${SEP}${RTK}cargo[[:space:]]+run([[:space:]]|\$)" \
   && printf '%s' "$CMD" | grep -qiE 'sim|animata'; then
  blocked="cargo run of the sim"
fi

[ -z "$blocked" ] && exit 0

# Conscious one-off bypass (consumed + logged).
ALLOW="$PROJECT_DIR/.claude/.sim-allow"
if [ -f "$ALLOW" ]; then
  rm -f "$ALLOW"
  printf '%s SIM-ALLOW bypass: %s\n' "$(date '+%F %T')" "$CMD" >> "$PROJECT_DIR/.claude/no-local-sim.log" 2>/dev/null || true
  exit 0
fi

REASON="BLOCKED (no-local-sim): $blocked. Executing the animata simulation locally is FORBIDDEN — the cloud is the authoritative gate. Push, then run 'bash scripts/ci-report.sh' (tests+goldens; exit 0 = green) or 'scripts/sim-run.sh <scenario>' (heavy runs). Allowed locally (compile-only): 'cargo test --no-run', 'cargo check', 'cargo clippy', 'cargo build'. One-off conscious bypass: 'touch .claude/.sim-allow' then re-run (consumed + logged in .claude/no-local-sim.log)."
python3 -c 'import json,sys; print(json.dumps({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":sys.argv[1]}}))' "$REASON"
exit 0
