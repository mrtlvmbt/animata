task: R-17 (render lane) — per-seed landform variety with free landform mixing
phase: CI
blocked_on: none — ready for CI
next: await CI pass; PM intake and merge to integration
updated: 2026-07-16 21:10

## Merged this session (integration branch render-r12-terragen-preview, head a78d808)
- R-14 (PR #441): AO + 45° thin bevel (bevel_drop=HEX_SIZE*BEVEL_FRAC, BEVEL_FRAC=0.08) + palette v2 + --bare + HEIGHT_SCALE=0.3 (user pick).
- R-16 (PR #445): brighter pastel palette (VALUE_STOPS floor 0.78, AMBIENT 0.55/DIFFUSE 0.45, lighter hues) + SINGLE-SOURCE
  world::palette::MATERIAL_COLORS (render + map_dump read one array). User accepted colours; PM did branch git-hygiene.
- Total merged slices: R-13, W-9, W-10, R-15a, R-14, R-16 (6). Remaining: R-15 (default-flip + 60fps gate).

## Process rules reinforced this session
- NEVER trust coder "pushed/done": confirm origin/<branch> HEAD moved to a NEW sha + verify content yourself. (B5 died+argued
  stale critic; B6 falsely "PR ready" while UNCOMMITTED; B7 falsely "clean" while cruft still tracked.)
- Coders sweep PM cruft via git-add-. in the shared worktree -> now blocked by .git/info/exclude patterns
  (/.ci-report*, /SWEEP_README.md, /.claude/plans/, /v2/.claude/, /docs/r16/). Verify branch diff is clean before merge.
- rev-parse origin/<branch> is AMBIGUOUS in this worktree -> use refs/remotes/origin/<branch> or explicit SHA.
