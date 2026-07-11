#!/usr/bin/env bash
# compile-check — the machine definition of "it compiles" for animata coders.
#
# WHY THIS EXISTS. The `STATUS: done` gate (`.claude/hooks/done-check.sh`) already blocks a false
# *final* report — it needs an open PR + green CI + closed checkboxes. But the recurring failure is
# EARLIER and cheaper: a coder claims "compiles ✓" *before pushing*, the push burns a ~15-minute CI
# round, and CI comes back with dozens of compile errors (typically stale `mutate()`/signature
# call-sites in TEST modules after a rebase from main). Root causes, both benign, both fixed here:
#   1. rtk proxy. The Bash hook rewrites the coder's TOP-LEVEL `cargo` calls and rtk swallows the
#      error detail, so a broken build reads as "ok" in the summary. cargo INSIDE a .sh is NOT
#      rewritten (see scripts/test-bar.sh header), so this script sees raw cargo output.
#   2. `cargo check` does not build test targets — exactly where the broken call-sites live. This
#      uses `cargo test --no-run`, which builds every test target WITHOUT running it: seconds, no
#      sim load, catches the whole class the CI round would have caught.
#
# This is NOT a heavy run and does NOT contradict "the authoritative green gate is cloud CI": it
# only answers "will the build + test targets COMPILE", the question a coder must answer locally
# before spending a CI round. The corridors/goldens still run in CI.
#
# stdout: PASS  |  FAIL: EXIT=<n> (compile) — followed by the raw cargo error tail.
# Exit:   0 = PASS, non-zero = the cargo exit code (FAIL-CLOSED — a missing toolchain is a FAIL,
#         never a silent pass). bash 3.2-safe.
#
#   scripts/compile-check.sh                 # whole workspace + all test targets
#   scripts/compile-check.sh -p animata-sim  # any cargo args pass through
set -u

command -v cargo >/dev/null 2>&1 || { echo "FAIL: EXIT=127 (compile) — cargo not on PATH (fail-closed)"; exit 127; }

ARGS=("$@")
[ ${#ARGS[@]} -eq 0 ] && ARGS=(--workspace --locked)

LOG="$(mktemp -t compile-check.XXXXXX)"
trap 'rm -f "$LOG"' EXIT

# Raw cargo (this runs inside a script → the rtk proxy does not touch it). --no-run builds the test
# harnesses but executes nothing, so this is compile-only and cheap.
cargo test --no-run "${ARGS[@]}" >"$LOG" 2>&1
EXIT=$?

if [ "$EXIT" -eq 0 ]; then
  echo "PASS"
  exit 0
fi

echo "FAIL: EXIT=$EXIT (compile) — raw cargo tail:"
# Surface the diagnostic that rtk would have swallowed: error lines + the count line.
grep -hE '^error(\[E[0-9]+\])?:|^error: aborting|could not compile' "$LOG" | sed 's/^/   /' | tail -n 40
echo "   ---"
tail -n 8 "$LOG" | sed 's/^/   /'
exit "$EXIT"
