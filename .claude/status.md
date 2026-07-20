task: #540 (Slice-1h: Plateau massif + anti-spike)
phase: CI
blocked_on: CI completion (golden-arm64, test-x86, v2-sim-x86, v2-golden-arm64)
next: await CI green, finalize PR (ready-for-review if all green)
updated: 2026-07-20 16:45

Completed:
- Plateau-core uplift profile: implement in generate_plate_uplift_field
  - CORE = belt_hw * 2/3 (plateau core, full amplitude)
  - Flank: ramp from CORE to belt_hw, linear decay to 0
  - Integer-only, correct ramp formula
- Flow-aware anti-spike: apply_plate_anti_spike() function
  - Spike_bound = 6 (max local step for isolated spike)
  - Isolation detection: 0 raised neighbors AND local_step > spike_bound
  - F3 NON-STRICT >= for crest test (protect broad plateaus)
- Gating: both changes inside enable_plate_sim guard (byte-identical default)
- Cold build: PASS (cargo clean -p world && compile-check.sh)
- Code-critic: PASS (removed dead raised_count variable)
- Render gallery: seed1.png (380.4K), seed2.png (357.3K) in .claude/w1h-gallery/
  - Both differ from Slice-1g (cmp output: char 35, line 3)
- PR #541: base terragen-v3, Closes #540
- Code-critic verdict: PASS (added as PR comment)
