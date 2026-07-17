task: U-11 granular loading progress — per-stage worldgen reporting (#478)
phase: tests/CI (awaiting green CI before ready-for-review)
blocked_on: CI run in progress (awaiting notification)
next: code-critic verdict; merge if all green; update DECISIONS.md
updated: 2026-07-17 14:30

## Implementation Summary
✓ Stage enum: 14 stages (heightfield, tectonics, erosion, ridges, aeolian, volcanic, glacial, coastal, beaches, talus, de-needle, classify, meshing, done)
✓ Russian labels: all 14 stages labeled for UI display
✓ Progress callback threaded through gen pipeline (observation-only, byte-pure)
✓ Callbacks at stage boundaries: classify_and_caps_with_callback implementation
✓ Render loader: displays Russian stage labels + permille progress (no redesign)
✓ Progress mapping: gen 0..800‰, meshing 800..1000‰
✓ Byte-purity test: progress_callback_byte_pure (world crate)
✓ Compile-check local: PASS (v2 workspace)
⏳ Code-critic self-review: in progress
⏳ CI gate: run #29610938537 in progress

## Code Changes (7 files)
- v2/crates/world/src/lib.rs: ProcgenWorld::new_with_callback, byte-purity test
- v2/crates/world/src/gen/caps.rs: classify_and_caps_with_callback with staged reporting
- v2/crates/render/src/world_spec.rs: 14-stage Stage enum, label_ru(), progress_permille()
- v2/crates/render/src/world_builder.rs: thread progress callback to world crate
- v2/crates/render/src/ui/loader.rs: display Russian stage labels
- v2/crates/render/src/main.rs: update progress mapping for all stages
- v2/crates/render/src/loader_state.rs: update Stage::GenerateHeightfield refs

## Gates (Pre-CI)
- Compile: PASS (local: compile-check ✓)
- Byte-purity: test included, queued for CI
- Render build: not tested locally (no-local-sim constraint)
- PM visual gate: screenshots deferred (no-local-sim constraint; PM can run with --slow-load --screenshot-loader seed=8)

## Scope Verification
✓ Progress sink: stages 0..11 in gen (no erosion/tectonics internals touched)
✓ Skipped landforms: report instantly (no stall)
✓ Render loader: Russian labels, permille bar, no redesign
✓ --slow-load: unchanged, honored via on_stage callbacks
✓ Byte-purity: callback observe-only (test verifies)
