#!/usr/bin/env bash
# done-gate — the coders' Stop hook (animata-followup plan 2026-07-06, step 2; consensus fixes
# F1/F5/F6/F7). Gates ONLY a final report carrying the literal line `STATUS: done`: runs
# done-check.sh and blocks a "done" without PR/CI/spec (the A-4 class). Mid-turn prose ("step 2
# done, moving on") and turns without a STATUS line are untouched. Strict order: the cheap pattern
# match runs BEFORE any network call. With stop_hook_active=true it does NOT block again (loop
# exclusion by design) but re-checks and writes a loud BLOCKED-OVERRIDE entry to
# .claude/done-gate.log — the PM treats that log as a hard-fail acceptance queue. Sanctioned
# one-shot escape: the file .claude/.done-allow (creating it is a visible action; it is consumed
# and logged, mirroring KIT_ALLOW_DIRTY). Enabled by the ANIMATA_DONE_GATE=1 slot (kit.config.sh,
# role scope A/B/C). No jq → no-op (exit 0), like every kit hook. bash 3.2-safe.
set -u

command -v jq >/dev/null 2>&1 || exit 0
PROJECT_DIR="${CLAUDE_PROJECT_DIR:-$(pwd)}"
# shellcheck disable=SC1091
[ -f "$PROJECT_DIR/.claude/kit.config.sh" ] && . "$PROJECT_DIR/.claude/kit.config.sh"
[ "${ANIMATA_DONE_GATE:-0}" = "1" ] || exit 0

INPUT="$(cat)"
STOP_ACTIVE="$(printf '%s' "$INPUT" | jq -r '.stop_hook_active // false')"
TRANSCRIPT="$(printf '%s' "$INPUT" | jq -r '.transcript_path // empty')"
[ -n "$TRANSCRIPT" ] && [ -f "$TRANSCRIPT" ] || exit 0

# Last assistant message (the transcript tail suffices — the final turn is always at the end).
LAST_TXT="$(tail -n 400 "$TRANSCRIPT" | jq -c 'select(.type=="assistant")' | tail -n 1 \
  | jq -r '.message.content[]? | select(.type=="text") | .text' 2>/dev/null)"
# F5: fire ONLY on the literal terminal token, never on prose.
printf '%s\n' "$LAST_TXT" | grep -q '^STATUS: done' || exit 0

LOG="$PROJECT_DIR/.claude/done-gate.log"
STAMP="$(date '+%Y-%m-%d %H:%M:%S')"

# Sanctioned one-shot escape (visible + logged + consumed).
if [ -f "$PROJECT_DIR/.claude/.done-allow" ]; then
  rm -f "$PROJECT_DIR/.claude/.done-allow"
  echo "$STAMP ALLOW: .done-allow consumed — done-gate deliberately bypassed" >> "$LOG"
  exit 0
fi

# Network calls only after the token matched (F6: zero network forks on ordinary turns).
CHECK_OUT="$(bash "$PROJECT_DIR/.claude/hooks/done-check.sh" 2>&1)" && exit 0

if [ "$STOP_ACTIVE" = "true" ]; then
  # F7: one-shot gate — a second block is forbidden (anti-loop), but the failure is LOUD on the
  # accepting side.
  echo "$STAMP BLOCKED-OVERRIDE: STATUS: done repeated after a block, done-check still FAIL → $CHECK_OUT" >> "$LOG"
  exit 0
fi

echo "$STAMP BLOCK: STATUS: done while done-check FAIL → $CHECK_OUT" >> "$LOG"
jq -n --arg reason "done-gate: the 'STATUS: done' report was rejected — $CHECK_OUT
Rule: 'done' exists only with an open PR, green CI and all spec checkboxes closed.
If the work genuinely is not at PR/CI yet — rewrite the final report as
'STATUS: blocked@<step>: <what is needed>' (an honest and permitted outcome).
Deliberate bypass (exception): create the file .claude/.done-allow and repeat — the bypass is logged." \
  '{decision: "block", reason: $reason}'
exit 0
