task: hex-diorama program (PM orchestration) — U-1 MERGED; U-2 in flight
phase: UI track. U-1 MERGED (PR #452 -> integration head e5c40fa): Panel/UiRoot/UiAction end-to-end (panel buttons actually work), wants_pointer/keyboard gating (CamInput injectable snapshot + pure apply_cam_input), v1 theme port, dead ColorMode+C deleted. Intake was 3 rounds: critic FAIL(F1 dead actions, F2 vacuous test) -> fixes -> PM-run test caught macroquad leak (headless panic) -> purge -> PM-run test caught assert/gate semantics -> fix -> test GREEN (PM-run) + pixels byte-identical (PM cmp) + critic PASS. U-2 DISPATCHED: issue #454, coder B16 (ad0a42e78c4638dab), branch u2-loading — WorldSpec/build_world worker + AppPhase + loader modal.
blocked_on: B16 U-2 (loader screenshot + 4 byte-identical harness pairs + D5 six-rule critic)
next: U-2 intake -> merge -> (U-3 chip+seed-regen ∥ U-4 zoom-to-cursor/left-drag) -> U-5 minimap -> R-15 (retained default + 60fps) -> final beauty pass -> user master-merge decision.
updated: 2026-07-17 01:40

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
