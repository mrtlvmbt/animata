<!-- claude-dev-kit overlay for skill `critique-delta`. Fill to ADAPT the kit base skill to THIS project.
     Empty (only comments) = base used as-is.
     Optional frontmatter (between --- lines): description OVERRIDES the base.
     Body below REPLACES the base default tail (below the kit:overlay marker) with
     project-concrete steps (the first-measurement commands, the classification/route table,
     guardrails). The base router intro/discipline above the marker is always kept.
-->

## Project glue (animata-pm)

For a **plan** under the consensus loop, the kit launcher runs the whole delta round for you AND keeps the
baseline — so the common path has **no `cp` and no `--delta` flag**. Prefer it over a hand-assembled
Agent-tool spawn, because it keeps the program-attested `.plan-consensus` marker and the opus fallback that
the raw `subagent_type: critic` path drops:

    .claude-dev-kit/bin/kit-critic .claude/plans/<name>.md .claude/.critic-prior.md

- **Artifacts.** Plans live at `.claude/plans/<name>.md`. The delta baseline is **automatic**: `kit-critic`
  snapshots each critiqued plan under `.claude/.consensus` (gitignored), keyed by the plan's path, and
  auto-deltas the next run on the same file against that snapshot. You keep no per-round snapshot files.
- **Findings.** The critic report lands at `.claude/.critic-report.md`; the machine-derived open-findings
  carry-file is `.claude/.critic-prior.md`
  (`python3 .claude-dev-kit/lib/critic-prior.py .claude/.critic-report.md > .claude/.critic-prior.md`).
- **Diff/ranges extraction AND baseline bookkeeping are the launcher's job now** — no `rtk`/git-tag/`cp`
  glue for plans. `kit-critic` does the diff → ranges → `[DELTA]` payload internally and re-anchors the
  baseline on every run. A git-tracked **doc** (not a plan) would instead use the manual `git diff <tag>`
  steps above. To pin a baseline by hand anyway: `kit-critic --delta <prior-plan> <plan> <prior-findings>`.
- **Sweep / disable.** `--full` forces a whole-plan round (the delta is a cheap first pass, not a full
  replacement — keep a periodic sweep); `KIT_CRITIC_AUTODELTA=0` disables auto-delta for a run.
- **Mode.** `kit-critic` is a cold fork (Mode B): for a recall-sensitive plan, invoke it k≥2 times and
  union the findings — one run per invocation; coldness is preserved each time.
