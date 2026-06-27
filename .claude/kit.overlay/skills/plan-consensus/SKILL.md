<!-- claude-dev-kit overlay for skill `plan-consensus`. Fill to ADAPT the kit base skill to THIS project.
     Empty (only comments) = base used as-is.
     Optional frontmatter (between --- lines): description OVERRIDES the base.
     Body below REPLACES the base default tail (below the kit:overlay marker) with
     project-concrete steps (the first-measurement commands, the classification/route table,
     guardrails). The base router intro/discipline above the marker is always kept.
-->

## Feeding the critic in THIS project (animata-pm)

Plans under consensus live at **`.claude/plans/<name>.md`** (e.g. `.claude/plans/substrate.md`). Model
and constraints come from `.claude/kit.config.sh` (`KIT_CRITIC_MODEL=opus`, `KIT_PLANNER_MODEL=opus`,
`KIT_CRITIC_CONSTRAINTS` = the animata sim-determinism / lane / golden / ТЗ envelope). The launcher is the
kit's **`.claude-dev-kit/bin/kit-critic`** (subscription session, $0) — NOT `pm/bin/code-review` (that is
the separate PR-vs-ТЗ gate run by `code-critic` after a PR exists; this loop runs on the *plan*, before
any code).

**The delta baseline is automatic — you keep no snapshot files.** `kit-critic` snapshots each critiqued
plan itself (under `.claude/.consensus`, gitignored), keyed by the plan's path, so round ≥2 re-critiques
only the DELTA with no `cp` and no `--delta` flag. Just call the same command each round:

**Round 1 — full cold critique** (no snapshot yet → whole plan):

    .claude-dev-kit/bin/kit-critic .claude/plans/<name>.md

Resolve every finding (Fix or Accept), editing `.claude/plans/<name>.md`. Then machine-derive the carry:

    python3 .claude-dev-kit/lib/critic-prior.py .claude/.critic-report.md > .claude/.critic-prior.md

**Round ≥2 — same command, auto-deltas** (a snapshot now exists → only the changed ranges + hunks go to
the critic, which Reads the enclosing sections itself):

    .claude-dev-kit/bin/kit-critic .claude/plans/<name>.md .claude/.critic-prior.md

The launcher diffs the prior snapshot against the current plan, sends the critic the path+ranges+hunks
(not the body), re-snapshots for the next round, and still writes the program-attested `.plan-consensus`
marker + keeps the opus fallback. If the plan is identical to the snapshot there is no change to validate
⇒ it exits 0 (you are already at the fixpoint). The report always lands at `.claude/.critic-report.md`.

Force a whole-plan **sweep** round (the delta is a cheap first pass, not a full replacement — keep a
periodic sweep for orthogonal bugs the change never reasons about) with `--full`; disable auto-delta for a
run with `KIT_CRITIC_AUTODELTA=0`. For a recall-sensitive plan, run k≥2 deltas and union their findings
(see the `critique-delta` skill).
