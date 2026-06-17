
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
