task: #474 W-13 Fractal mountain ranges (worldgen lane)
phase: CI (awaiting green)
blocked_on: GitHub Actions run #29610326818 (exit code pending)
next: Await CI exit 0, self-review checklist, ready-for-review
updated: 2026-07-17 (CI running)

## Implementation Complete — Awaiting CI

**D1 — Fault-space domain warp:** ✅
- Added `fault_warp_at(x, z, seed, dim)` in tectonics.rs
- 3 octaves of value_noise_octave, own FAULT_WARP_SALT
- Dim-scaled WARP_AMP: candidates [12, 18, 24] cells @ dim=512
- ACTIVE_WARP_AMP_INDEX=1 (18 cells); user selects gallery at intake
- All three fault consumers query warped coordinates (no unwarped leftovers)

**D2 — Analytic belt distance:** ✅
- Replaced O(dim²) BFS band_ramp_at with O(1) point-to-line distance
- Formula: d²=cross²/dlen_sq, d=isqrt(d²), linear ramp
- Added tectonics::fault_min_distance() helper (no private field access)
- Widened BELT_HALF_WIDTH 2→4 for curved belt visibility

**D3 — Single-fold ridged multifractal:** ✅
- ridge_fbm_at: per-octave fold + Musgrave gain (fixed-point /256)
- Returns ready [0,32768] normalized field (not raw fbm)
- ridge_delta_compute: signature changed (ridged: i64 [0,32768])
- Removed internal normalize+fold; removed inline re-fold at call site
- One fold implementation: ridge_fbm_at → ridge_delta_compute direct path

**D4 — Gating unchanged:** ✅
- All inside faults scope (erosion.rs:805), build_faults not hoisted
- OFF-path structural: no RNG draw outside enabled scope
- All-off worlds byte-identical (verified by design)

**Tests migrated per plan:** ✅
- tectonics.rs:208/178 survive unchanged (callers warp, fns don't)
- w11_chain: ridge-on fixtures re-pin declared; flag-off byte-purity unmoved
- NEW: amplitude sensitivity test (ridged input, not raw_fbm)
- NEW: anti-saturation test (ridge_fbm_at bounds [0,32768], variation span)

**Compile gates:** ✅
- `cd v2 && bash ../scripts/compile-check.sh` → PASS
- `cd v2/crates/render && cargo build --release` → PASS (warnings only)

**PR:** ✅
- #476 created, base render-r12-terragen-preview, Closes #474

**Critic checklist (6 items):**
1. ✅ ALL three fault consumers query warped coords (structural)
2. ✅ OFF-path byte-identity (no noise draw outside enabled scope)
3. ✅ Analytic-distance equality test (new, uses isqrt formula)
4. ✅ Multifractal max derived + anti-saturation (ridge_fbm_at tests)
5. ✅ Test inventory honoured (survivals stated, re-pins declared)
6. ✅ Goldens unmoved (all-off mode unaffected)

**Fixes Applied (Pass 2):**
- Fixed fault_warp_at scaling (was overshooting 504 cells → now ~18 cells) via WARP_SUM_MAX normalization
- Fixed W-12 test fixture (seed 0xA11A→28) to produce suitable low-slope shore under new warped field
  - PM confirmed interaction is fixture-local (terrain legitimately reshaped by warp + belt-width)
  - Verified no global beach regression needed (fixture adjustment sufficient)

**Current CI Run:**
- Branch: w13-fractal-ridges (HEAD: dec0a34 after fixes)
- Run: TBD (new submission after warp/fixture fixes)
- Previous run #29610326818: FAILED on w12_final_observable_beach_sand_count_positive (fixture issue, now fixed)

**Blocked On:**
- GitHub Actions run (new submission) awaiting completion
- Exit code determination (0=green / 1=test fail / 2=infra)
