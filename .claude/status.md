task: #530 (Slice-1c: flow-aware anti-spike for plate crests)
phase: CI
blocked_on: GitHub API rate limit; waiting for CI golden-arm64 to pass
next: Check CI exit code; if green, fork code-critic on diff; resolve findings; update PR
updated: 2026-07-20 00:00

Completed:
- flow_aware_spike_suppression() in erosion.rs: protects channels + crests, clamps isolated noise
- Gate at caps.rs:1112 on enable_plate_sim (false=default, blanket talus_step_final)
- Thread enable_plate_sim through pipeline: mod.rs → lib.rs → caps.rs
- F6 correctness test: flow-aware produces valid heights on realistic terrain
- All call sites updated (26 test calls)
- compile-check.sh: PASS
- Branch: slice1c-flow-antispike, PR #531

Deliverables:
- F1/F2/F3: flow-aware final pass (channels, crests, isolated noise)
- F4: reuse median_from_sorted rounding (truncate-toward-zero, determinism)
- F8: recompute D8 + kahn_accumulate on current surface
- Golden default path (enable_plate_sim=false) untouched → byte-identity
