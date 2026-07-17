task: hex-diorama program (PM orchestration) — U-3 MERGED; U-5 (final UI slice) in flight
phase: UI track. U-3 MERGED (PR #459 -> integration head 21c7983): in-game reseed (N key + panel button, Procgen+standalone only) + progress chip (Panel) + --regen-to; PM-verified determinism gate GREEN both paths (regen-to-X == cold-launch-X byte-identical), 36/36 tests, chip PNG by PM eyes. Intake scars: B20 died (context) losing work; B21 confused root-cause narrative + tried deferring the chip twice (rejected — chip IS the user's feature #2); gate was RED at first PM run (20.8% pixels) -> stale-seed plumbing fixed. U-5 DISPATCHED: issue #460, coder B22 (a44e9eaaa49b0e32e), branch u5-minimap — cell_color downsample + viewport quad + click-to-jump.
blocked_on: B22 U-5 code-critic review + CI test verification (byte-identical harness + R-13 parity)
next: code-critic findings -> fix or accept -> merge to render-r12-terragen-preview -> UI track CONCLUSION.
updated: 2026-07-17 14:45

## Merged (integration branch render-r12-terragen-preview, head 21c7983)
R-13, W-9, W-10, R-15a, R-14, R-16, R-17, U-0, U-1, U-2, U-3, U-4.
User's five UI features: loading screen ✓, in-game indicator ✓ (chip), zoom ✓, drag ✓; minimap (U-5) in flight.

## Standing process rules (session scars) — unchanged, see memory coder-verification-contract
- remote sha + branch-contains + symbol grep + PM-run tests + PM-eyes PNGs; PM test bypass = touch
  PM/.claude/.sim-allow in a SEPARATE call; visual features need in-app framebuffer captures;
  cmp misses double-drawing; kit-hook push false-positive -> PM pushes.
