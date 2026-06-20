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

Run tests through **`./scripts/test-bar.sh`**, never bare `cargo test`. It wraps `cargo test`, streams
a progress indicator (so the human sees how far a long run is), and passes failure detail through
(panic body, assert `left:`/`right:` — needed when re-pinning the golden checksum).

- Full suite: `./scripts/test-bar.sh` (defaults to `--release --workspace`).
- Filtered: `./scripts/test-bar.sh -p animata-sim --release state_checksum` (any `cargo test` args pass through).
- It runs raw `cargo test` internally (bypasses the rtk proxy that otherwise swallows test output), and
  honours `.cargo/config.toml`'s `RUST_TEST_THREADS=1`. In a non-TTY context (a captured/backgrounded
  run) it prints periodic checkpoint lines instead of a `\r` bar — set cadence with `BAR_EVERY=N`.
- The canonical green gate is still a full **`--release`** run (acceptance corridors are tuned there).
