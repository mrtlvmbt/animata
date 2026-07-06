#!/usr/bin/env bash
# done-check — the machine definition of DONE for animata coders (animata-followup plan 2026-07-06,
# step 2). PASS only if: (a) an OPEN PR exists for the current branch; (b) CI on HEAD is green (no
# "CI is expected, run-id ..." escape — a two-pass task reports `STATUS: blocked@N: awaiting CI` on
# pass 1); (c) the PR body has no unchecked spec boxes (`- [ ]`).
# FAIL-CLOSED: any failure of the check itself (no gh, no auth, network, detached HEAD) = FAIL with
# a reason — the gate only blocks the word "done"; the cost of a false block is rephrasing the
# report as blocked@.
# Invocation: standalone (by the coder before the final report) or from done-gate.sh.
# stdout: PASS | FAIL: <reasons>.  Exit: 0 = PASS, 1 = FAIL.  bash 3.2-safe.
set -u

fail() { echo "FAIL: $*"; exit 1; }

command -v gh >/dev/null 2>&1 || fail "gh is not installed — cannot verify PR/CI (fail-closed)"
command -v jq >/dev/null 2>&1 || fail "jq is not installed — cannot parse CI status (fail-closed)"

BRANCH="$(git branch --show-current 2>/dev/null || true)"
[ -n "$BRANCH" ] || fail "detached HEAD — no branch, no done"
case "$BRANCH" in main|master) fail "no done reports from $BRANCH — work happens on a feature branch" ;; esac

PR_JSON="$(gh pr view --json number,state,body,statusCheckRollup 2>&1)" \
  || fail "no PR found for branch '$BRANCH' (gh: $(printf '%s' "$PR_JSON" | head -1)) — this is exactly the A-4 case"

STATE="$(printf '%s' "$PR_JSON" | jq -r '.state')"
[ "$STATE" = "OPEN" ] || fail "PR is $STATE, not OPEN"

# CI: every check in the rollup must have finished successfully (SUCCESS/NEUTRAL/SKIPPED).
BAD="$(printf '%s' "$PR_JSON" | jq -r '[.statusCheckRollup[]?
        | select((.conclusion // .state // "PENDING") | test("SUCCESS|NEUTRAL|SKIPPED") | not)
        | (.name // .context // "check")] | join(", ")')"
[ -z "$BAD" ] || fail "CI not green: $BAD (waiting for CI → report 'STATUS: blocked@N: awaiting CI', not done)"
N_CHECKS="$(printf '%s' "$PR_JSON" | jq -r '[.statusCheckRollup[]?] | length')"
[ "${N_CHECKS:-0}" -gt 0 ] || fail "no CI checks on HEAD — run the pipeline (push → ci-report.sh)"

# Spec items: an unchecked checkbox in the PR body = an unfinished criterion.
UNCHECKED="$(printf '%s' "$PR_JSON" | jq -r '.body' | grep -c '^\s*- \[ \]' || true)"
[ "${UNCHECKED:-0}" -eq 0 ] || fail "PR body has $UNCHECKED unchecked spec item(s) (- [ ])"

echo "PASS"
exit 0
