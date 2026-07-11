# animata — project instructions

**User-facing language is Russian.** The kit hook `kit-user-lang` (configured with `KIT_USER_LANG='Russian'` in
`.claude/kit.config.sh`) enforces Russian replies in the main conversation thread; this configuration is
re-injected into every prompt and does not drift. All internal artifacts (this file, control files,
agent-traffic handoff blocks) and all inter-agent communication are English. Code, identifiers, branch/commit/PR
names, git command bodies, and machine tokens (F-ids, severities, VERDICT strings) are always English.

## When you hit a wall — check `GOTCHAS.md` FIRST

When something breaks or behaves unexpectedly, consult **[`GOTCHAS.md`](GOTCHAS.md)** (repo root) BEFORE
debugging from scratch — it is a symptom→cause→fix index of rakes already stepped on (rtk summary lies,
`cargo build` vs `cargo test --no-run`, arm64 golden re-pin, `--admin` merge auth, …). After you solve a
NEW wall/surprise, append ONE compressed entry (`Symptom → Cause → What to do`) to its `## Gotchas`
section — append-only, never rewrite existing entries.

<!-- claude-dev-kit:rules START (managed — do not edit by hand) -->
## Working with claude-dev-kit (consumer contract)

This repo consumes the **claude-dev-kit** sub-repo at `.claude-dev-kit/` (agents, gate hooks, agent-metrics).
The kit is **read-only** here — it is a shared mechanism layer, not project code.

- **Never edit `.claude-dev-kit/**`** (a guard hook blocks it). Fixes to the kit go upstream in its own
  repo, then `git submodule update --remote .claude-dev-kit && .claude-dev-kit/install.sh`.
- **Enrich/override an agent** → edit `.claude/kit.overlay/agents/<name>.md` (tools are UNIONed with
  the base; description/model/skills override; the body replaces the base output skeleton and may add
  project grounding). Re-run `.claude-dev-kit/install.sh` to regenerate `.claude/agents/<name>.md`.
- **Project agents that have no kit base** → write them straight into `.claude/agents/`; the kit
  leaves them alone.
- **Change behavior/slots** (lint cmd, protected branches, intent triggers, metrics dir, agent
  format-contract headers `KIT_AGENT_FMT_*`) → edit `.claude/kit.config.sh`.
- Generated agents ARE committed (the effective prompt is reviewable); the overlay + config remain
  the source of truth. After editing an overlay, **re-run install.sh** — a commit gate
  (kit-generated-guard) blocks commits when a generated agent drifts from its overlay/base.
<!-- claude-dev-kit:rules END -->

## Decision index — `pm/reports/DECISIONS.md` (mandatory upkeep)

`pm/reports/DECISIONS.md` is the PM-workspace map of every mechanic → status (LIVE / GATED-OFF /
CUT / NULL-MARGINAL) → code anchor (flag/const) → justification doc → verdict. It exists so a reviewer or
anyone revising an algorithm can answer "why is this here / why did we NOT do X" without re-deriving. It
is the durable counterpart to the PM-private memory narrative.

**Where reports are version-controlled:** `pm/` is its OWN git repo (`git@github.com:mrtlvmbt/animata-pm.git`),
nested inside and gitignored by the animata code repo (`.git/info/exclude` → `/pm/`). So reports ARE in
git history — just of the SEPARATE `animata-pm` repo, never the code repo. `main` there is protected
(same as animata): commit on a branch → PR → merge. Writing a report to `pm/reports/` is NOT enough; it
must be committed+pushed to animata-pm or it exists only on this machine.

**Upkeep is a rule, not a courtesy:**
- **On every slice MERGE** — PM adds/updates its row (flag + report + status). A merged slice with no
  DECISIONS row is an incomplete handoff.
- **On every ladder / frontier CONCLUSION** (a verdict landmark lands) — PM appends the CONCLUSION: flip
  the status to its final verdict (FAITHFUL / NULL / MARGINAL / ARTEFACT-CUT), link the landmark, and
  record the one-line root-cause + the anti-knob decision (what we DECLINED to crank and why). This is
  how the "why we stopped" survives past the session.
- **Commit+push to animata-pm in the same pass** — the DECISIONS row edit and any new/updated report go
  onto a branch in the `pm/` repo → PR → merge. A report written but left uncommitted in animata-pm is a
  half-done handoff (durable only on this machine).
- Keep anchors at **flag/const/report granularity** (line numbers drift; flags and report names don't).
- Landmarks (`*-landmark.md`) remain the authoritative long-form "why"; DECISIONS.md is the index INTO
  them. When you write a landmark, add/flip its DECISIONS row in the same pass.

## Running tests (ALL agents — mandatory)

**The authoritative green gate is the cloud CI pipeline, NOT a local run.** The heavy suite (the
8000-tick acceptance corridors) is offloaded to GitHub Actions so it never taxes the dev machine.
**Precondition (per host): `gh` is installed and authenticated** (`gh auth login`, scope `repo`) —
`ci-report.sh` preflights this and tells you exactly what to fix if it's missing. The standard loop is
**commit → `git push` → `bash scripts/ci-report.sh`**:

- `ci-report.sh` finds the run for HEAD, waits for it, and exits **0 = all green / 1 = tests failed /
  2 = infra/timeout**. The exit code is the signal; on failure read `.ci-report/failed.log` (panic
  body, assert `left:`/`right:`) and `.ci-report/artifacts/*/junit.xml` (which tests failed).
- **Merge ONLY when `ci-report.sh` exits 0.** That replaces the old "run the full `--release` suite
  locally" gate. Do NOT run the full `./scripts/test-bar.sh` suite locally — that is exactly the
  machine-load CI exists to remove.
- CI is four jobs (determinism is per-arch). Two guard the **v1 `animata-sim`** regression shield:
  `test-x86` (ubuntu, the corridors + everything except the 3 exact-golden tests) and `golden-arm64`
  (macos-latest, matched arch, the 3 `state_checksum`/golden locks). Two guard the **active v2 program**
  (`v2/crates/**` — `sim-core`/`cli`/…, where all current mechanic work lands): `v2-sim-x86` (invariants +
  R14 1-vs-N) and `v2-golden-arm64` (the `v2_golden_conserved_*` determinism pins). The overall run is
  green iff **all four** pass. The render bin is deliberately out of CI, so UI/render changes still verify
  locally (clippy + in-app — see the `animata-ui` skill).
  > ⚠ The rest of this section (re-pin / `golden.rs` / `test-bar.sh`) still describes the **v1**
  > `animata-sim` machinery; the v2 goldens are `v2_golden_conserved_*`. The v1↔v2 testing contract wants a
  > dedicated refresh — this note only corrects the "two jobs / animata-sim only" claim, which was false.
- **Re-pinning the golden:** read the new `left:`/`right:` from `.ci-report/failed.log` (the
  `golden-arm64` job), not a local run.

**Heavy simulations AND any new test/check run in the CLOUD, not on the dev machine.** Long headless
runs, perf benchmarks at scale, high-population timing, parameter sweeps, multi-seed probes → dispatch
via **`scripts/sim-run.sh <scenario> [k=v …]`** (the manual `sim-run.yml` pipelines:
`evo-stats`/`perf`/`v2-perf`/`multiseed`/`sweep`/`gridsweep`), which waits and fetches the result (it preflights the `gh`
`workflow` scope and tells you if it's missing). When you ADD a new test or acceptance check, land it in
the suite and let the CI gate run it (push → `ci-report.sh`) — don't burn the dev machine verifying it
locally. The cloud is the default execution surface for anything heavy or new.

**Observational runs PARALLELISE — experiments don't have to be serial.** GitHub Actions runs
dispatches concurrently (no `concurrency:` gate), so independent probes (different seeds / params /
scenarios) can run at once: either a grid inside ONE `sweep`/`multiseed` dispatch (serial cells), a
`gridsweep` dispatch (each value on its own runner, concurrently), or several
`scripts/sim-run.sh … &` backgrounded together (each writes a per-nonce `.sim-run/<nonce>/`, so
parallel fetches don't collide). **This is ONLY for observational sim-runs and independent
experiments.** The determinism golden + acceptance corridors stay single-writer (animata-sim skill §9):
never race two agents on one golden-touching change — that is unattributable drift, not parallelism.

**Local `./scripts/test-bar.sh` stays available but OPTIONAL — only for fast targeted iteration** on a
single test while developing (e.g. `./scripts/test-bar.sh -p animata-sim --release state_checksum`); it
is NOT the gate. It wraps `cargo test` (never bare `cargo test`), runs raw cargo internally (bypasses
the rtk proxy that swallows test output), honours `.cargo/config.toml`'s `RUST_TEST_THREADS=1`, and
passes failure detail through; in a non-TTY run it prints checkpoint lines instead of a `\r` bar
(cadence `BAR_EVERY=N`).

## Final-report contract (coders A/B/C — mandatory)

The last line of every session/dispatch final report must be exactly one of:

```
STATUS: done
STATUS: blocked@<step>: <what is needed to continue>
```

- `STATUS: done` is permitted ONLY when `.claude/hooks/done-check.sh` prints `PASS`: an open PR on
  the current branch + green CI on HEAD + zero unchecked acceptance items (`- [ ]`) in the PR body.
  The stop-hook `done-gate.sh` validates this automatically and blocks a false `done` claim (origin: case
  A-4 — placeholder pin pushed, PR not created, reported done; PM fixed it by hand).
- Two-pass tasks (value is born in CI: golden re-pin and kin) report pass 1 as
  `STATUS: blocked@N: awaiting CI (pass 2 of 2)` — this is honest and the gate permits it.
- "Done" mid-session ("step 2 done, moving on") is not a final report; the gate does not touch it.
- Intentional gate bypass: create `.claude/.done-allow` and retry — the bypass is one-shot and logged
  in `.claude/done-gate.log` (mirrors `KIT_ALLOW_DIRTY`).

## Pre-merge self-review (coders A/B/C — mandatory)

Before moving a PR to ready-for-review, fork the **`code-critic`** agent (in `.claude/agents/`,
model opus) as a cold review of the branch diff against the issue ТЗ/acceptance criteria:
feed it `git diff main...HEAD` + the ТЗ text. Rules:

- `VERDICT: FAIL` with an unresolved `bug` or unguarded `robustness` → fix BEFORE ready-for-review,
  do not argue with the cold fork in chat (Fix-or-Accept; Accept only for `tradeoff`/`style`).
- Append the verdict block (final lines, including `VERDICT:`) as a PR comment — PM sees that
  self-review happened and what was resolved.
- This does NOT replace the PM run at intake: the coder layer catches cheap early wins (same-tier,
  own blind spots); the PM layer remains authoritative — the same layering as done-gate + PM lint.
- The critic MUST check against the "Known review false-positives" list below — cite the precedent,
  do not re-derive.

## Status file (coders A/B/C — end of every session/dispatch)

The last action of a session is to write `.claude/status.md` (machine-readable, not versioned):

```
task: <#issue / PR / short name>
phase: <current: plan | code | tests | CI | PR>
blocked_on: <what exactly is blocking>
next: <first action of next session>
updated: <YYYY-MM-DD HH:MM>
```

PM reads these files (`../A/.claude/status.md` etc.) INSTEAD of asking "where did you stop";
`updated` older than 24h = stale, PM asks directly. This does NOT replace the `STATUS:` line of
the final report — they are separate channels (report = handoff, file = live state between sessions).

## Handoff intake (PM — machine lint)

- A coder's final report without a `^STATUS: (done|blocked@…)` line is an **automatic return**
  ("format violated"), content is not parsed. A missing token = rejection, not silent pass.
- Before accepting "done", PM inspects the coder's `.claude/done-gate.log`: an entry `BLOCKED-OVERRIDE`
  (repeated `STATUS: done` after a block) or a fresh `ALLOW` triggers a hard-fail: handoff is not
  accepted until the cause is reviewed.

## Known review false-positives (ground-checked — cite precedent, do not re-derive)

Code reviews by coders (agent and manual) repeatedly re-open the same NON-bugs. Each entry carries
both evidence AND a precondition — the entry is valid only while its precondition holds (a falsifiable
prior, as in the kit: `.claude-dev-kit/docs/review-agent-accuracy.md` § Version-floor check).

- **Editing a golden constant in a test CREATED by the same session/branch is NOT goldbricking.**
  Canonical golden-test creation for new deterministic code: placeholder → first run → pin actual
  value (precedent: session B 1f4485b8, w3/w4/w5_chain, 03.07.2026 — two independent review forks
  falsely flagged it). *Precondition:* the test file is new in this branch (not in main). If the pin
  for an EXISTING golden test in main is edited without justification of an algorithm change, the
  flag is valid.
- **A fix-loop "test fails → Edit → test passes" with transparent reasoning between runs is NOT fraud.**
  Normal development; a 18–44 second interval between fail and pass is an edit, not falsification
  (precedent: session A f19d3c4b, 5 false cases from "strict" review). *Precondition:* there is an
  Edit/text explaining the reasoning between runs. Silent assertion rewrites without justification —
  the flag is valid.
- **A `v2_golden_drift` fall locally on arm64 during an intentional hash-contribution change is an
  expected re-pin, not a hidden break.** Goldens are arch-bound (see the header of
  `crates/cli/tests/golden.rs`: arm64 CI job only, x86 excluded, debug self-skip); re-ping by process —
  from `.ci-report/failed.log` (`golden-arm64` job), owner PM. *Precondition:* the golden.rs header
  preserves the arch-bound contract and the hash-logic change is declared in the PR.
