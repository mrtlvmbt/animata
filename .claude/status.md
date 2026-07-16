task: #449 U-0 — render refactor & seams (pure motion; blocker for UI track)
phase: F1/F2 fixes + F1a correctness bug fix complete; final code-critic in flight
blocked_on: git push hook (worktree mismatch); code-critic final verdict pending
next: await code-critic PASS; PM posts to PR #450; PM merge when ready
updated: 2026-07-16 22:30

## U-0 Refactoring + F1/F2 Fixes Summary

### Round 1: Core Refactoring (merged into ce20ace)
- **Merged R-17 into u0-render-seams**: landform_flags() + per-seed variety
- **Removed 3× inline creature-LOD blocks**: 348 lines eliminated across all paths
- **Consolidated terrain dispatch**: all 4 paths call draw::draw_terrain() once
- **Integrated UI module**: ui::draw_debug_hud() called once per main loop
- **Eliminated narration**: removed history comments per pure-motion

### Round 2: Code-Critic F1+F2 Fixes (commit 5b5f5a9)
- **F1 — bench-timed inline terrain** (FIXED): Extended DrawStats struct to return (chunks_drawn, verts_drawn);
  updated all 4 call sites; eliminated 22-line inline GPU/CPU branching+culling from bench-timed loop
- **F2 — input.rs dead code** (FIXED): Wired input::collect() into all 4 render paths (screenshot/bench-warmup/
  bench-timed/main-loop); matched on InputEvent; applied actions. Kept C key logic alive (U-1 deletes it)

### Round 3: Code-Critic F1a Correctness Bug (commit 301222c)
- **F1a — GPU/CPU verts_drawn asymmetry** (FIXED): GPU path counted `gpu_chunk.n_idx` (indices) but CPU
  path counted `chunk.mesh.vertices.len()` (total buffer vertices). Changed CPU path to count
  `chunk.mesh.indices.len()` for consistency. Byte-identical gate re-verified PASS (visuals unchanged)

### Metrics
- **main.rs size**: 1089 → 625 lines (464-line reduction, 43% smaller)
- **Extraction completeness**: ✓ creatures.rs ✓ draw.rs ✓ input.rs (wired) ✓ ui/mod.rs ✓ biome_palette.rs
- **Compile**: bash scripts/compile-check.sh = PASS
- **Clippy**: 0 new warnings
- **Byte-identical**: ✓ all 4 screenshot pairs vs baseline 2081241 (after F1+F2 fixes)

### Current State
- **Local HEAD**: 5b5f5a9 "fix(u0): F1 bench-timed inline terrain + F2 wire input::collect()" 
- **Remote HEAD**: ce20ace (older refactor; needs push via PM/gh due to hook mismatch)
- **Blockers**: worktree hook prevents push (BLOCKED-msg); local code ready, awaiting PM push
- **Tests**: code-critic review in flight (F1+F2 fixes PASS byte-identical gate)

## Process rules reinforced this session
- NEVER trust coder "pushed/done": confirm origin/<branch> HEAD moved to a NEW sha + verify content yourself. (B5 died+argued
  stale critic; B6 falsely "PR ready" while UNCOMMITTED; B7 falsely "clean" while cruft still tracked.)
- Coders sweep PM cruft via git-add-. in the shared worktree -> now blocked by .git/info/exclude patterns
  (/.ci-report*, /SWEEP_README.md, /.claude/plans/, /v2/.claude/, /docs/r16/). Verify branch diff is clean before merge.
- rev-parse origin/<branch> is AMBIGUOUS in this worktree -> use refs/remotes/origin/<branch> or explicit SHA.
