#!/usr/bin/env bash
# Progress wrapper over `cargo test`. Runs raw cargo (no rtk filtering — the hook only rewrites
# Claude's own top-level Bash calls, not commands inside a script), so per-test lines stream and
# progress is visible. Tests are serial here (.cargo/config.toml pins RUST_TEST_THREADS=1 for
# macroquad's global RNG), so progress advances one test at a time and never goes backwards.
#
# VERDICT AND COUNTS ARE AUTHORITATIVE — both come from cargo (exit code + its `test result:` line),
# NOT from line-parsing: under `--nocapture` a test's `... ok`/`... FAILED` interleaves with its
# stderr (panic body / eprintln) on the same line, so the live counter is best-effort only and is
# used solely to drive the running %. The final pass/fail verdict and totals are cargo's own.
#
# Live progress is ALSO mirrored to a fixed file so any shell can watch it regardless of how this
# script's stdout is routed (e.g. when an agent runs it backgrounded):
#       tail -f /tmp/animata-test.log        # override with BAR_FILE=/path
#
# Render modes (auto-detected): TTY → a single \r-updated bar; pipe → newline checkpoints.
# Force with BAR_MODE=tty|plain. Cadence BAR_EVERY (default 1 = a line per test, for responsive tail).
#
#   scripts/test-bar.sh                       # full release suite (--release --workspace)
#   scripts/test-bar.sh -p animata-sim rng    # any cargo-test args pass through
set -uo pipefail

ARGS=("$@")
[ ${#ARGS[@]} -eq 0 ] && ARGS=(--release --workspace)

MODE=${BAR_MODE:-$([ -t 1 ] && echo tty || echo plain)}
EVERY=${BAR_EVERY:-1}
PROGRESS=${BAR_FILE:-/tmp/animata-test.log}
LOG=$(mktemp -t test-bar.XXXXXX)
trap 'rm -f "$LOG"' EXIT
: >"$PROGRESS"   # truncate the mirror so a tail shows only this run

echo "building + counting tests…" >&2
TOTAL=$(cargo test "${ARGS[@]}" -- --list 2>/dev/null | grep -cE ': test$')
[ "${TOTAL:-0}" -eq 0 ] && TOTAL=1
echo "running $TOTAL tests (${ARGS[*]})  ·  live: tail -f $PROGRESS" | tee -a "$PROGRESS" >&2

WIDTH=24
SPIN='⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏'; si=0
done=0; pass=0; fail=0; ign=0; cargo_exit=1

draw() { # $1 = last test name, $2 = result
  local pct=$(( done * 100 / TOTAL ))
  # Always mirror a newline checkpoint to the fixed file (so `tail -f` works in any shell).
  if [ "$2" = FAILED ] || (( done % EVERY == 0 || done == TOTAL )); then
    printf '[%3d%%] %d/%d  ✓%d ✗%d ⊘%d  %s\n' "$pct" "$done" "$TOTAL" "$pass" "$fail" "$ign" "$1" >>"$PROGRESS"
  fi
  if [ "$MODE" = tty ]; then
    local frac=$(( done * WIDTH / TOTAL )) b="" i
    for ((i=0; i<WIDTH; i++)); do [ $i -lt $frac ] && b+="█" || b+="░"; done
    local c=${SPIN:si:1}; si=$(( (si + 1) % ${#SPIN} ))
    printf '\r\033[K%s [%s] %3d%%  %d/%d  ✓%d ✗%d ⊘%d  %.38s' \
      "$c" "$b" "$pct" "$done" "$TOTAL" "$pass" "$fail" "$ign" "$1"
  elif [ "$2" = FAILED ] || (( done % EVERY == 0 || done == TOTAL )); then
    printf '[%3d%%] %d/%d  ✓%d ✗%d ⊘%d  %s\n' "$pct" "$done" "$TOTAL" "$pass" "$fail" "$ign" "$1"
  fi
}

# Process-substitution (not a pipe) so the counters survive the loop. The trailing sentinel carries
# cargo's real exit code out of the subshell — the authoritative pass/fail signal.
while IFS= read -r line; do
  if [[ $line == __CARGO_EXIT__* ]]; then cargo_exit=${line#__CARGO_EXIT__}; continue; fi
  printf '%s\n' "$line" >>"$LOG"          # tee full output for end-of-run extraction
  if [[ $line =~ ^test\ (.+)\ \.\.\.\ (ok|FAILED|ignored) ]]; then
    case "${BASH_REMATCH[2]}" in
      ok) ((pass++)) ;; FAILED) ((fail++)) ;; ignored) ((ign++)) ;;
    esac
    ((done++)); draw "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}"
  fi
done < <(stdbuf -oL cargo test "${ARGS[@]}" -- --nocapture 2>&1; echo "__CARGO_EXIT__$?")

[ "$MODE" = tty ] && printf '\r\033[K'

# Test diagnostics (eprintln means, golden left:/right:) — useful for tuning even on a green run.
grep -hE 'mean|correlation|component|nutrient:|left:|right:' "$LOG" | sed 's/^/   · /' || true
# cargo's own summary line(s) — authoritative counts.
grep -hE '^test result:' "$LOG" | sed 's/^/   /'

# Authoritative totals summed from cargo's `test result:` lines (lib + doctests + integration).
sum_field() { grep -hoE "[0-9]+ $1" "$LOG" | awk '{s+=$1} END{print s+0}'; }
apass=$(sum_field passed); afail=$(sum_field failed); aign=$(sum_field ignored)

if [ "$cargo_exit" != 0 ]; then
  verdict="✗ cargo exit=$cargo_exit — $afail failed, $apass passed, $aign ignored — FAILURES:"
  echo "$verdict"; echo "$verdict" >>"$PROGRESS"
  grep -hE '^\s+[a-z_]+::' "$LOG" | grep -vE '\.\.\.' | sed 's/^/   ✗ /' | sort -u | tee -a "$PROGRESS"
  exit "$cargo_exit"
fi
verdict="✓ all green — $apass passed, $aign ignored"
echo "$verdict"; echo "$verdict" >>"$PROGRESS"
