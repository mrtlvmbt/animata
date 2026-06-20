#!/usr/bin/env bash
# Progress wrapper over `cargo test`. Runs raw cargo (no rtk filtering — the hook only rewrites
# Claude's own top-level Bash calls, not commands inside a script), so per-test lines stream and
# progress is visible. Tests are serial here (.cargo/config.toml pins RUST_TEST_THREADS=1 for
# macroquad's global RNG), so progress advances one test at a time and never goes backwards.
#
# VERDICT IS AUTHORITATIVE: it comes from cargo's real exit code (≠0 on ANY failure), NOT from
# line-parsing — under `--nocapture` a panicking test's `... FAILED` interleaves with stderr and
# can't be counted reliably, so never trust the parsed counter for pass/fail.
#
# Two render modes (auto-detected):
#   • TTY  → a single \r-updated compact bar  (interactive terminal)
#   • pipe → periodic newline checkpoints      (captured/tailed output, e.g. Claude's Bash tool)
# Force a mode with BAR_MODE=tty|plain. Checkpoint cadence: BAR_EVERY (default 5 tests).
#
#   scripts/test-bar.sh                       # full release suite (--release --workspace)
#   scripts/test-bar.sh -p animata-sim rng    # any cargo-test args pass through
set -uo pipefail

ARGS=("$@")
[ ${#ARGS[@]} -eq 0 ] && ARGS=(--release --workspace)

MODE=${BAR_MODE:-$([ -t 1 ] && echo tty || echo plain)}
EVERY=${BAR_EVERY:-5}
LOG=$(mktemp -t test-bar.XXXXXX)
trap 'rm -f "$LOG"' EXIT

echo "building + counting tests…" >&2
TOTAL=$(cargo test "${ARGS[@]}" -- --list 2>/dev/null | grep -cE ': test$')
[ "${TOTAL:-0}" -eq 0 ] && TOTAL=1
echo "running $TOTAL tests (${ARGS[*]})" >&2

WIDTH=24
SPIN='⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏'; si=0
done=0; pass=0; fail=0; ign=0; cargo_exit=1

draw() { # $1 = last test name, $2 = result
  local pct=$(( done * 100 / TOTAL ))
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

if [ "$cargo_exit" != 0 ]; then
  echo "✗ cargo exit=$cargo_exit — FAILURES:"
  grep -hE '^\s+[a-z_]+::' "$LOG" | grep -vE '\.\.\.' | sed 's/^/   ✗ /' | sort -u   # the `failures:` list
  exit "$cargo_exit"
fi
echo "✓ all green  $pass passed, $ign ignored  ($TOTAL total)"
