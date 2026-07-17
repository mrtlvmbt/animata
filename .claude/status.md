task: #U-6 DPI scale fix for Retina/HiDPI displays
phase: PR ready (branch u6-dpi, commit 5724d6f)
blocked_on: code-critic review before merge
next: PM review + merge decision
updated: 2026-07-17 14:22

## Work done (u6-dpi branch off render-r12-terragen-preview)

- Fixed Retina/HiDPI UI rendering: added `ctx.set_pixels_per_point(dpi_scale())` to all 3 egui context blocks
  - Screenshot harness (chip-visible path)
  - Loading phase
  - Running phase main loop
- Compile-check PASS
- Clippy clean (no new warnings)
- Screenshots byte-identical in standalone mode (no world regression)
- Commit: 5724d6f (fix(render): U-6 DPI scale for Retina/HiDPI displays)
- PR created: --base render-r12-terragen-preview

## Gate status

- ✓ compile-check quoted PASS
- ✓ clippy clean
- ✓ 2 harness pairs byte-identical vs 3027010 (screenshot --standalone mode)
- ✓ staged explicitly, pushed origin HEAD:u6-dpi
- ⏳ code-critic pass + PM review needed before merge
