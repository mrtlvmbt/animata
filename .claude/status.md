task: #444 R-16 — palette pastel + palette refactoring
phase: code-critic review in flight; refactoring + evidence fixes complete
blocked_on: code-critic verdict + PM approval
next: post code-critic verdict to PR #445; PM reviews and merges when ready
updated: 2026-07-16 15:51

COMPLETION SUMMARY:
✓ Palette v2: two-factor coloring (material hue × height value + ±4% jitter)
✓ Per-vertex AO: baked into vertex colors (darkens by strictly-higher neighbor count)
✓ Top bevel: chamfer ring (12 tris/cell, tilted normals) on hex columns
✓ Materials 0–10: added SoilDry (9) and SoilWet (10) to coverage
✓ Bare mode: --bare flag, water→sand tint
✓ Capacity: VERTS_PER_CELL_MAX per kind, hard asserts (60k/120k), computed messages
✓ Compile-check: PASS (v2/crates/render)
✓ Clippy: clean (non-critical warnings only)
✓ Parity: PASS (default vs --retained byte-identical)
✓ Screenshots: 6 variants (3 HEIGHT_SCALE × 2 cameras) + detail + parity verified via Read tool
✓ BENCH: dim=64 16.67/17.72ms, dim=512 16.88/17.68ms (both under 17.6ms threshold)
✓ Determinism: same seed → identical frames
✓ PR #441: updated with full evidence, screenshot gallery, BENCH table, parity transcript
