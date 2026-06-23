# animata — инструкции проекта

**Язык общения — РУССКИЙ.** Отвечай пользователю по-русски во всех сессиях этого репозитория. Это
относится к главному треду диалога. Исключения (всегда по-английски): код и идентификаторы, имена
веток/коммитов/PR и тела git-команд, машинные токены инструментов. Форк-агенты кита
(`bug-hunt`/`subsystem-reviewer`/`web-research`/`critic`/`judge`) уже локализованы на русский через
`.claude/kit.overlay/agents/` — их вывод тоже русский, кроме машинных токенов (`F1`, `[severity: …]`,
`VERDICT: PASS|FAIL`).

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
- CI is two jobs (determinism is per-arch — see the memory [[ci-push-triggered]]): `test-x86`
  (ubuntu, the corridors + everything except the 3 exact-golden tests) and `golden-arm64`
  (macos-latest, matched arch, the 3 `state_checksum`/golden locks). It covers **`animata-sim` only**.
  The `animata` render bin is deliberately out of CI, so UI/render changes still verify locally
  (clippy + in-app — see the `animata-ui` skill).
- **Re-pinning the golden:** read the new `left:`/`right:` from `.ci-report/failed.log` (the
  `golden-arm64` job), not a local run.

**Heavy simulations AND any new test/check run in the CLOUD, not on the dev machine.** Long headless
runs, perf benchmarks at scale, high-population timing, parameter sweeps, multi-seed probes → dispatch
via **`scripts/sim-run.sh <scenario> [k=v …]`** (the manual `sim-run.yml` pipelines:
`evo-stats`/`perf`/`multiseed`/`sweep`), which waits and fetches the result (it preflights the `gh`
`workflow` scope and tells you if it's missing). When you ADD a new test or acceptance check, land it in
the suite and let the CI gate run it (push → `ci-report.sh`) — don't burn the dev machine verifying it
locally. The cloud is the default execution surface for anything heavy or new.

**Observational runs PARALLELISE — experiments don't have to be serial.** GitHub Actions runs
dispatches concurrently (no `concurrency:` gate), so independent probes (different seeds / params /
scenarios) can run at once: either a grid inside ONE `sweep`/`multiseed` dispatch, or several
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
