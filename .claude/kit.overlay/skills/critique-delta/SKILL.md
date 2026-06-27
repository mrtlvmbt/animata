<!-- claude-dev-kit overlay for skill `critique-delta`. Fill to ADAPT the kit base skill to THIS project.
     Empty (only comments) = base used as-is.
     Optional frontmatter (between --- lines): description OVERRIDES the base.
     Body below REPLACES the base default tail (below the kit:overlay marker) with
     project-concrete steps (the first-measurement commands, the classification/route table,
     guardrails). The base router intro/discipline above the marker is always kept.
-->

## Project glue (animata-pm)

For a **plan** under the consensus loop, the kit launcher runs the whole delta round for you — prefer it
over a hand-assembled Agent-tool spawn, because it keeps the program-attested `.plan-consensus` marker and
the opus fallback that the raw `subagent_type: critic` path drops:

    .claude-dev-kit/bin/kit-critic --delta <prior-round-snapshot> <current-plan> <prior-findings>

- **Artifacts.** Plans live at `.claude/plans/<name>.md`. The explicit delta baseline (a plan has no
  mid-loop commit) is a per-round snapshot at `.claude/plans/.consensus/<name>.r<N>.md` (gitignored) —
  `cp` the plan there right after each round's critique so the baseline is exactly the text that was
  critiqued. That `cp` is the project's "re-anchor the baseline" step (step 5 above); there is no git tag.
- **Findings.** The critic report lands at `.claude/.critic-report.md`; the machine-derived open-findings
  carry-file is `.claude/.critic-prior.md`
  (`python3 .claude-dev-kit/lib/critic-prior.py .claude/.critic-report.md > .claude/.critic-prior.md`).
- **Diff/ranges extraction is the launcher's job now** — no `rtk`/git-tag glue needed for plans.
  `kit-critic --delta` does steps 1–3 (diff → ranges → `[DELTA]` payload) internally. A git-tracked **doc**
  (not a plan) would instead use the manual `git diff <tag>` steps above.
- **Mode.** `kit-critic --delta` is a cold fork (Mode B): use it for round 1's revisions and for
  large/churning plans. For a recall-sensitive plan, invoke it k≥2 times and union the findings — the
  launcher is one run per invocation; coldness is preserved each time.
