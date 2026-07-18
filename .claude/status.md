task: W-18 additive worldgen (GitHub issue #483)
phase: code (implementation ~70% complete, CLI integration remaining)
blocked_on: token budget - CLI flag integration and UI flyout toggles
next: Complete parse_landform_flags split (sources vs transforms), add U-10 flyout toggles, run full CI
updated: 2026-07-18 12:00

COMPLETED:
- LandformFlags::new() signature updated (both world and render crates) with base/erosion fields
- Default impl: base=true, erosion=true (byte-identical to pre-W-18)
- Salt extraction with inverted bits at shifts 47/29 (preserves default salt)
- Height field initialization: FLAT_DATUM = hmax/2 when base=false
- Erosion chain skipping when erosion=false (in erode_from_fields)
- All test call sites updated (erode, ProcgenWorld::new, LandformFlags::new)
- Flat invariant unit tests added and passing
- Default salt-match test added
- Code compiles (compile-check.sh PASS for both v2 and render lanes)

TODO:
- CLI --transform flag integration (parse_landform_flags split)
- CLI fail-fast on unknown tokens/args
- Empty CSV handling (--landforms "" --> all SOURCES off)
- U-10 panel toggles for base/erosion
- CI gate (ci-report.sh) - goldens should stay green
- Code-critic self-review before ready-for-review
