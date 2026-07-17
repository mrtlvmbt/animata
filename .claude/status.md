task: U-10 — user-controlled landform toggles at world generation
phase: complete — ready for PM intake
blocked_on: none
next: PM intake review
updated: 2026-07-17 21:30

## Completion Summary

All 5 acceptance items completed and verified locally. Branch u10-landform-toggles ready for PM intake.

**Tests Passed:**
- ✓ Determinism: --landforms tect,coastal --seed 42 (cold) vs regen path = BYTE-IDENTICAL 447K PNG
- ✓ Byte-purity: Default pipeline unchanged from base 810a121 = BYTE-IDENTICAL 1.2M PNG
- ✓ Evidence PNGs: 3 screenshots in docs/u10/ (all verified via Read)
- ✓ Code-critic review: VERDICT PASS (all criteria met, no viable failure points)
- ✓ Compile-check: PASS
- ✓ Clippy: Clean (no new warnings)

**Commits:**
1. 931c2a8 — Core implementation (F1/F3/F4 fixes)
2. 7448886 — F2 fix (N-key parity with manual flags)
3. dc92327 — Evidence screenshots (3 PNGs)

**Features Delivered:**
- Ландшафты section: 7 checkboxes + авто/вручную toggle
- Auto mode: read-only seed-derived flags (visually disabled)
- Manual mode: user-editable with live dependency clamps
- Empty-set hint: "пусто → тектоника включится сама"
- Regen parity: N-key and button both use manual flags
- CLI flag: --landforms <csv> (no panic on invalid)
- Determinism: explicit flags override WorldSpec
- No world-crate changes (render-side only)

**Known Limitations:**
- None (F1, F2, F3, F4, F5 all resolved)

**Code-critic Status:**
- Posted to PR #473 as comment
- VERDICT: PASS
- Ready for PM intake

**Git Status:**
- Branch: u10-landform-toggles
- Base: render-r12-terragen-preview (810a121)
- Local HEAD: dc92327
- Ready to push and close for PM review
