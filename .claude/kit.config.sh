#!/usr/bin/env bash
# claude-dev-kit slot config. Copy to your project as `.claude/kit.config.sh`; the kit's hooks
# source it (falling back to these defaults if absent). One source of project-specific values —
# the hooks themselves stay generic.

# ── Branch policy (branch-guard) ───────────────────────────────────────────────
# Space-separated protected branches that block direct commit/push.
KIT_PROTECTED_BRANCHES="main master"

# ── Commit gate (commit-gate) ──────────────────────────────────────────────────
# Fast lint/typecheck run before a commit, ONLY when matching files are staged. Keep it FAST — heavy
# whole-project builds belong in CI (plugin/ci/) or pre-push, not on every commit.
# Rust:    "cargo clippy --all-targets -- -D warnings"     glob: '\.rs$'
# Android: "./gradlew lint"  (add detekt/ktlintCheck only if applied) glob: '\.kts?$'
# Node:    "npm run lint"                                   glob: '\.(ts|tsx|js|jsx)$'
KIT_LINT_CMD="cargo clippy --all-targets -- -D warnings"
KIT_LINT_STAGED_GLOB='\.rs$'
# Set to 1 to skip the gate entirely.
KIT_LINT_DISABLED=0
# Changed-set: commit-gate exports $KIT_CHANGED_FILES (newline-separated staged files, glob-filtered)
# before running KIT_LINT_CMD, so a monorepo can lint only affected modules. The kit supplies the set;
# YOUR command derives modules (it must not bake topology into the kit). Example for a Gradle monorepo:
#   KIT_LINT_CMD='mods=$(printf "%s\n" "$KIT_CHANGED_FILES" | sed -E "s#/src/.*##" | sort -u); \
#                 for m in $mods; do ./gradlew ":${m//\//:}:ktlintCheck" || exit 1; done'
# Emergency escape: prefix a commit with KIT_ALLOW_DIRTY=1 to bypass a failing gate — it is LOGGED to
# .claude/kit-dirty-escapes.log and CI still re-checks (unlike the invisible `git commit --no-verify`).

# ── Generated-agent freshness (kit-generated-guard, local = advisory) ──────────
# Generated agents/skills (from base+overlay) are committed; on commit this hook only WARNS on drift
# (it never blocks and never `git add`s — a hook must not mutate your staged draft). The AUTHORITATIVE
# freshness check is the CI tier (plugin/ci/kit-ci.sh — wire via install.sh --ci). 1 to disable the warn
# (permanent, unlogged — by design, like KIT_LINT_DISABLED).
KIT_GENERATED_GUARD_DISABLED=0
# No-CI consumers: turn the advisory warn into a REAL local deny. 1 = deny a commit on drift.
KIT_GENERATED_GUARD_DENY=0
# Sanctioned per-commit escape from DENY mode (the rushed-commit case): set KIT_ALLOW_DRIFT=1 on the
# one commit → no deny, and the escape is LOGGED to .claude/kit-dirty-escapes.log (governance trail,
# mirrors KIT_ALLOW_DIRTY). The permanent kill is KIT_GENERATED_GUARD_DISABLED above — that one is NOT
# logged, on purpose: a flipped kill switch is the human's call, the same as `git commit --no-verify`.

# ── agent-metrics location (agent-metrics-record) ──────────────────────────────
# Path (relative to project root) to the metrics dir holding SCHEME_VERSION + runs.tsv.
KIT_METRICS_DIR="tools/agent-metrics"

# ── Intent router (intent-router) ──────────────────────────────────────────────
# Regex of bug-shaped phrases that trigger a nudge toward the debug-triage entry point.
KIT_INTENT_TRIGGERS='не работает|broken|glitch|doesn'\''t work|crash|exception|regression|тормоз|дрож|артефакт'
# Message injected when a trigger fires. Point it at YOUR debugging spine.
KIT_INTENT_MSG='Looks like a bug report. Before hypothesizing, run the canonical first measurement from your debugging spine, then route to the right diagnose path. Do NOT guess the cause before measuring.'

# ── Debugging spine path (referenced by agents/docs) ───────────────────────────
KIT_DEBUG_SPINE="docs/debugging-spine.md"

# ── Critic agent + plan-consensus loop (critic, bin/kit-critic, plan-consensus-guard) ──
# Model the `critic` fork runs at. CAPABILITY-ASYMMETRY RULE: keep the critic one tier ABOVE the model
# that AUTHORED the plan — WHEN a tier above exists. A same-model critic shares the author's training
# priors, so it inherits the author's blind spots — the cold fork removes *motivated* blindness
# (hope-echoing, scope-creep) but NOT *correlated* blindness (a flaw the model can't see when generating
# it also can't see when critiquing). NOTE (2026-06): Anthropic withdrew claude-fable-5, so OPUS IS THE
# CEILING and the critic defaults to opus. An opus planner gets a same-tier critic — expected, not a
# misconfig; kit-critic's note_top_tier says so each run. Close the residual correlated blindness by hand
# (human second pass, or a second fork seeded with the diff not the plan). Do NOT drop the critic BELOW
# the author's tier to run faster. (Same rule governs `judge` below; policy shared in lib/model-tier.sh.)
KIT_CRITIC_MODEL='opus'
# Used — loudly, on stderr — if KIT_CRITIC_MODEL is unavailable on this session (forking a newer model
# can fail). Degrades the critic instead of dropping it. Keep this >= the planner's tier if you can.
KIT_CRITIC_MODEL_FALLBACK='opus'
# The model that AUTHORS plans in this project (the planner). kit-critic compares it (by capability
# tier — "opus" matches "claude-opus-4-8") to the critic model and warns UP FRONT if they're equal:
# a same-tier critic shares the author's blind spots, so its approval is worth little. Leave empty if
# unknown — the check is skipped and only a softer fallback heads-up remains. Best results: have the
# planner export KIT_PLANNER_MODEL=<its real model> for the run (it knows its own model); this static
# value is the weaker fallback (it can't see a session forced onto a different model than usual).
KIT_PLANNER_MODEL='opus'
# Platform/toolchain limits the critic holds every plan against (the [TARGET SYSTEM CONSTRAINTS] block
# bin/kit-critic feeds it). Make this YOUR real environment contract.
KIT_CRITIC_CONSTRAINTS='animata Rust workspace (animata-sim lib + animata render bin). Fixed-timestep, seeded sim: bit-exact DETERMINISM is a hard invariant, locked by a per-arch golden state_checksum (golden-arm64 on macos); any plan touching sim state must keep the checksum reproducible or explicitly re-pin the golden. Authoritative test gate = cloud CI (ci-report.sh exit 0), which covers animata-sim ONLY — the render bin is verified locally (clippy + in-app), never in CI. Heavy or new runs (perf, sweeps, multi-seed, long headless) dispatch to the cloud via sim-run.sh, never the dev machine. Lanes sim|infra|render with single-writer ownership; golden-touch changes need an explicit re-pin step. Every plan maps to its ТЗ acceptance criteria.'
# plan-consensus-guard (PreToolUse ExitPlanMode): WARNs (never blocks) if no fresh consensus marker.
KIT_PLAN_CONSENSUS_DISABLED=0
KIT_PLAN_CONSENSUS_TTL_MIN=30

# ── Judge agent (judge, bin/kit-judge) ─────────────────────────────────────────
# The evaluative twin of the critic: scores an ARTIFACT against a RUBRIC → PASS/FAIL. The SAME
# capability-asymmetry rule applies — keep the judge one tier ABOVE the model that AUTHORED the output
# it scores WHEN a tier above exists (a same-tier judge shares the author's blind spots + self-preference
# bias; its PASS is worth little). Mirrors the critic's policy: post-fable (2026-06) opus is the ceiling,
# so the judge defaults to opus — note_top_tier flags the same-tier scoring on each run.
KIT_JUDGE_MODEL='opus'
# Used — loudly, on stderr — if KIT_JUDGE_MODEL is unavailable on this session. Degrades, doesn't drop.
KIT_JUDGE_MODEL_FALLBACK='opus'
# The model that AUTHORED the artifact under test. kit-judge compares it (by capability tier) to the
# judge model and warns UP FRONT if equal. Leave empty if unknown — the precise check is skipped and
# only a softer fallback heads-up remains. Best supplied per-run by the caller (it knows the author);
# this static value is the weaker fallback. Mirror of KIT_PLANNER_MODEL.
KIT_AUTHOR_MODEL='opus'

# ── Context meter (kit-context-meter) ──────────────────────────────────────────
# This PM session runs on the 1M-token window, so the hook's 200k default would read ~90% at ~180k
# (really ~18%) and cry "/compact NOW" falsely — observed live (181867/200000). Pin the real window so
# occupancy is honest. The meter then stays quiet below KIT_CTX_WARN_PCT; manual /compact discipline at
# ~200k is unaffected. Set back to 200000 (or unset) only if this clone ever runs the 200k window.
KIT_CTX_LIMIT=1000000
KIT_CTX_WARN_PCT=55

# ── Kit sub-repo location (kit-readonly-guard) ─────────────────────────────────
# Path (relative to project root) of the consumed kit submodule. The guard hook blocks Edit/Write
# into it — project changes go in the overlay / this config, not the kit. install.sh sets this.
KIT_SUBREPO_DIR=".claude-dev-kit"

# ── Agent format-contract headers (agent-metrics-record) ───────────────────────
# Per-agent required output headers, '|'-separated. Lets an overlay localize the skeleton so the
# metrics drift-check matches your agents' real output. Var name = KIT_AGENT_FMT_<name with - as _>.
# Defaults (English) live in the hook; override only when your overlay changes the skeleton/language.
# Russian skeletons — match the animata overlays in .claude/kit.overlay/agents/ (kit talks RU here).
KIT_AGENT_FMT_bug_hunt='## Гипотеза|## Кандидаты|## Следующий шаг'
KIT_AGENT_FMT_subsystem_reviewer='## Подсистема|## Вердикт'
KIT_AGENT_FMT_web_research='## Ответ|## Источники'
KIT_AGENT_FMT_critic='## Иллюзия|## Точка отказа|## Реальность железа|## Альтернативный паттерн'
KIT_AGENT_FMT_judge='## Вердикт|## По критериям|VERDICT:'

# ── auto-detected by install.sh (overrides the defaults above) ──
KIT_LINT_CMD='cargo clippy --all-targets -- -D warnings'
KIT_LINT_STAGED_GLOB='\.rs$'
KIT_METRICS_DIR='tools/agent-metrics'
KIT_SUBREPO_DIR='.claude-dev-kit'
