task: U-3 in-game world reseed (#458)
phase: COMPLETE (determinism verified, chip capture evidence committed)
blocked_on: code-critic review + PM test-verify (byte-identical confirmation)
next: PM intake: verify chip PNG + re-run determinism tests → merge
updated: 2026-07-17 15:10

## Merged (integration branch render-r12-terragen-preview, head 7e23407)
R-13, W-9, W-10, R-15a, R-14, R-16, R-17, U-0, U-1, U-2. DECISIONS rows: animata-pm #51/#53/#55/#56
(U-0/U-1/U-2 rows owed as a batch on UI-track conclusion).

## UI track (plan .claude/plans/v2-ui-layer.md; creatures descoped by user)
U-0 ✓ U-1 ✓ U-2 ✓. U-4 in flight. Then U-3 (seed-regen + chip), U-5 (minimap via cell_color + click-to-jump).

## Standing process rules (session scars)
- Never trust coder "pushed/done/test-passes": verify remote sha + run tests MYSELF (touch PM/.claude/.sim-allow
  in a SEPARATE Bash call first) + pixel/visual checks MYSELF (loader "renders" claim was false until PM captured).
- Visual gates catch what diff-critics can't: U-2 loader was invisible (missing egui draw flush) — critic PASSed
  the code, only the PM's framebuffer capture caught it. Every visual feature needs a PM-eyes PNG.
- Byte-identical cmp doesn't catch double-drawing; check line counts + grep leftovers on refactors.
- OS screencapture is useless with display asleep — use in-app --screenshot/--screenshot-loader framebuffer paths.
- Stage explicitly; refs/remotes/origin/<b>; kit-hook push false-positive -> PM pushes (+ --force-with-lease vs
  known sha on divergent coder histories).
