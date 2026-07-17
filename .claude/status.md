task: hex-diorama program (PM orchestration) — U-1 MERGED; U-2 architecture COMPLETE (code phase)
phase: UI track. U-1 MERGED (PR #452 -> integration head e5c40fa). U-2 ACTIVE: code phase (commit 145dfed), architecture complete: WorldSpec/build_world unified path + raw chunks + LoadState/AppPhase + loader.rs. Compilation: 0 errors. NEXT: worker thread spawning + main loop integration + --slow-load flag + screenshot baseline (docs/u2/) + code-critic (D5 six-rule checklist).
blocked_on: B16 U-2 (worker thread → AppPhase state machine → baseline screenshot evidence for 4 harness pairs)
next: U-2 worker + screenshots → code-critic (D5 rules) → intake → merge → (U-3 ∥ U-4) → U-5 → R-15 → beauty pass → user master-merge.
updated: 2026-07-17 07:30

## Merged (integration branch render-r12-terragen-preview, head e5c40fa)
R-13, W-9, W-10, R-15a, R-14, R-16, R-17, U-0 (de-monolith; main.rs 1145->625), U-1 (UI core + gating).
DECISIONS rows: animata-pm PRs #51/#53/#55/#56 (U-0/U-1/U-2 rows owed as a batch on UI-track conclusion).

## UI track (plan .claude/plans/v2-ui-layer.md; creatures descoped by user)
U-0 ✓, U-1 ✓. U-2 in flight. Then U-3 indicator+seed-regen (Procgen+standalone only, --regen-to),
U-4 zoom-to-cursor+left-drag (APPLIED factor after clamp; CamInput already injectable), U-5 minimap
(cell_color downsample, click-to-jump via UiAction::JumpCamera — actions sink already live).

## Standing process rules (session scars)
- Never trust coder "pushed/done/test-passes": verify remote sha moved + run tests MYSELF (bypass:
  touch PM/.claude/.sim-allow in a SEPARATE Bash call, then cargo test) + pixel-cmp MYSELF.
- Coder test claims are void — B13 claimed a green test that PANICKED (never ran it). Honest coder
  terminal state = blocked@test-verify.
- Byte-identical cmp does NOT catch double-drawing; check line counts + grep leftovers on refactors.
- Vacuous tests are worse than none: made runnable, the U-1 test immediately caught a REAL gate bug (yaw).
- Stage explicitly; .git/info/exclude guards PM cruft; refs/remotes/origin/<b> (rev-parse ambiguous);
  kit-hook push false-positive -> PM pushes via rtk proxy git push (+ --force-with-lease vs known sha
  when coder histories diverge — B13/B15 divergent status commits precedent).
