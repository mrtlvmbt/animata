task: #534 (Slice-1e: terragen-v3 render gallery)
phase: PR (ready-for-review)
blocked_on: none
next: await review/merge
updated: 2026-07-20 15:32

Completed:
- CLI flag: --plate-sim added to VALID_FLAGS, CliArgs, parse_args
- WorldSpec: enable_plate_sim field added
- build_world: accepts enable_plate_sim parameter, threads to both Procgen branches
- All 5 build_world call sites updated with spec.enable_plate_sim
- Compile: PASS (cargo build --release -p render, 7 crates)
- Clippy: 0 errors
- Screenshots: 2 captured (plate-sim-1.png, plate-sim-2.png @ 2048×1536 each)
- PR #535 created on terragen-v3 base, closes #534
