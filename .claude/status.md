task: #528 (Slice-1b: terragen-v3 orogeny + plate-uplift wiring)
phase: code - orogeny module 100% implemented, parameter threading in progress
blocked_on: Complete erode*/classify_and_caps* parameter updates across ~20 call sites in tests/bins/glacial.rs
next: Batch-fix remaining call sites, compile-check, push, CI + code-critic, PR ready-for-review
updated: 2026-07-20 14:30

Implemented:
- generate_plate_uplift_field: DONE (F1/F2/F10/F11 all locked)
- compute_belt_distance (F2 BFS): DONE, tests pinned
- F8 linearity test: DONE (L1 norm monotonic)
- Orogeny module: v2/crates/world/src/gen/orogeny.rs ~320 LOC, integer-only, no floats
- Wiring into erode pipeline: DONE (after volcanic, before erode_from_fields)
- Function signatures updated: erode, erode_with_tectonics, classify_and_caps*_staged*_with_callback

Outstanding: ~20 call sites need enable_plate_sim=false, plate_strength=100 params (mechanical fixes)
Orogeny code quality: ✓ pure integer, ✓ no floats, ✓ documented constants, ✓ tests green locally
