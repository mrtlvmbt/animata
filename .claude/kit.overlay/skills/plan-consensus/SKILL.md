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
`KIT_CRITIC_CONSTRAINTS`). The launcher is the kit's **`.claude-dev-kit/bin/kit-critic`** (subscription
session, $0) — NOT `pm/bin/code-review` (that is the separate PR-vs-ТЗ gate run by `code-critic` after a
PR exists; this loop runs on the *plan*, before any code).

**Round snapshots = the delta baseline.** A plan has no commit of its own mid-loop, so keep each
critiqued round's text as an explicit baseline file under `.claude/plans/.consensus/` (gitignored,
transient). Snapshot the plan *right after* each round's critique, so the baseline is exactly the text
that was critiqued.

    mkdir -p .claude/plans/.consensus

**Round 1 — full cold critique** (you need the baseline):

    .claude-dev-kit/bin/kit-critic .claude/plans/<name>.md
    cp .claude/plans/<name>.md .claude/plans/.consensus/<name>.r1.md      # snapshot what was critiqued

Resolve every finding (Fix or Accept), editing `.claude/plans/<name>.md`. Then machine-derive the carry:

    python3 .claude-dev-kit/lib/critic-prior.py .claude/.critic-report.md > .claude/.critic-prior.md

**Round ≥2 — feed the DELTA, not the whole plan** (see the `critique-delta` skill for the contract):

    .claude-dev-kit/bin/kit-critic --delta .claude/plans/.consensus/<name>.r$((N-1)).md \
        .claude/plans/<name>.md .claude/.critic-prior.md
    cp .claude/plans/<name>.md .claude/plans/.consensus/<name>.r$N.md     # re-anchor for the next round

`--delta` diffs the prior-round snapshot against the current plan and sends the critic only the changed
ranges + hunks (it Reads the enclosing sections itself), while still writing the program-attested
`.plan-consensus` marker and keeping the opus fallback. If the plan is identical to the snapshot there is
no change to validate ⇒ `kit-critic` exits 0 (you are already at the fixpoint). The report always lands
at `.claude/.critic-report.md`. For a recall-sensitive plan, run k≥2 deltas and union their findings.

NOTE: `KIT_CRITIC_CONSTRAINTS` in `.claude/kit.config.sh` currently holds the generic kit default — set it
to the animata-pm plan envelope (sim determinism / lane ownership / golden-touch / ТЗ acceptance) when a
plan's survival depends on those, so the cold critic stress-tests against the real constraints.
