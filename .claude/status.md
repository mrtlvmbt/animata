task: hex-diorama program (PM orchestration) — U-3 MERGED; U-5 (final UI slice) in flight
phase: U-5 VERIFICATION (PR #461, branch u5-minimap HEAD 299d8cd). Addressed PM feedback on viewport-quad unproven + click-to-jump unverified: added --jump-to <x>,<z> flag (fires UiAction::JumpCamera after startup, same as minimap click, enables visual verification); added 3x unit tests for corner projection + camera-pan (minimap_view_proj_transforms_screen_corners, minimap_viewport_quad_offsets_with_camera_pan, minimap_uv_world_mapping_is_invertible); added detailed SAFETY invariant comment on unsafe cache pointer. Code-critic PASS (3 findings fixed). Deliverables: raster (per-map relief band [p2,p98]), viewport quad (corner projection), click-to-jump, caching. Verified: compile-check PASS, tests compile, clippy clean. Awaiting PM screenshot proof (jump_before/after PNGs) + byte-identical harness CI.
blocked_on: PM screenshot verification (viewport quad movement + jump success) + CI byte-identical + R-13 parity
next: merge to render-r12-terragen-preview -> UI track CONCLUSION.
updated: 2026-07-17 15:50

## Merged (integration branch render-r12-terragen-preview, head 21c7983)
R-13, W-9, W-10, R-15a, R-14, R-16, R-17, U-0, U-1, U-2, U-3, U-4.
User's five UI features: loading screen ✓, in-game indicator ✓ (chip), zoom ✓, drag ✓; minimap (U-5) in flight.

## Standing process rules (session scars) — unchanged, see memory coder-verification-contract
- remote sha + branch-contains + symbol grep + PM-run tests + PM-eyes PNGs; PM test bypass = touch
  PM/.claude/.sim-allow in a SEPARATE call; visual features need in-app framebuffer captures;
  cmp misses double-drawing; kit-hook push false-positive -> PM pushes.
