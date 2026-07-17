task: hex-diorama program (PM orchestration) — U-3 MERGED; U-5 (final UI slice) in flight
phase: UI track. U-3 MERGED (PR #459 -> integration head 21c7983). U-5 READY FOR CI (PR #461, branch u5-minimap): minimap raster (D6 palette-match via per-map relief band) + viewport quad (D7 camera frustum) + click-to-jump (UiAction). Code-critic self-review: 3 findings (hardcoded relief band, sampling coords, missing quad) ALL FIXED + verified (compile-check PASS, clippy clean). Recompile commit 4b749e25ed (code-critic fixes). Ready for PM intake + byte-identical harness CI + R-13 parity.
blocked_on: PM intake + CI verification (v2-render lane, R-13 parity gate, byte-identical 256/seed=1 both paths)
next: wait for CI green -> merge to render-r12-terragen-preview -> UI track CONCLUSION.
updated: 2026-07-17 15:30

## Merged (integration branch render-r12-terragen-preview, head 21c7983)
R-13, W-9, W-10, R-15a, R-14, R-16, R-17, U-0, U-1, U-2, U-3, U-4.
User's five UI features: loading screen ✓, in-game indicator ✓ (chip), zoom ✓, drag ✓; minimap (U-5) in flight.

## Standing process rules (session scars) — unchanged, see memory coder-verification-contract
- remote sha + branch-contains + symbol grep + PM-run tests + PM-eyes PNGs; PM test bypass = touch
  PM/.claude/.sim-allow in a SEPARATE call; visual features need in-app framebuffer captures;
  cmp misses double-drawing; kit-hook push false-positive -> PM pushes.
