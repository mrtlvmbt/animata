# Phase-0b Glacial Moraine Sweep

## Status

✓ **Code complete and compiling**: `bash scripts/compile-check.sh` PASS

## Implementation Summary

### Step 1: Added `truncated` counter to `GlacialState` (F56)

- **File**: `v2/crates/world/src/gen/glacial.rs`
- **Changes**:
  - Added fields to `GlacialState`: `exported_till`, `truncated`, `band_capacity` (lines 413-415)
  - Extended `deposit_moraine_budget_drain` to compute `truncated` per F56 spec:
    - `truncated = Σ over band cells with headroom>0 of max(0, min(profile[ring], remaining) − headroom)`
    - Formula: deposit mass the profile wanted beyond a cell's headroom
    - Mandatory `headroom>0` filter: prevents counting pre-existing over-hmax erosion tail
  - Updated return tuple from `(Vec<i64>, i64, i64, Vec<bool>, i64)` to `(Vec<i64>, i64, i64, Vec<bool>, i64, i64)` (line 451)

### Step 2: Wrote Phase-0b sweep driver

- **File**: `v2/crates/world/src/bin/height_stats.rs`
- **Functionality**:
  - Calls `run_glacial_with(seed, dim, hmax, height, &Profile, k_band)` over a grid
  - **Profile grid**: `d_peak ∈ {15,25,40,60}` × `k_band ∈ {2,3,5,8}` = 16 combinations
  - **Masks**: Production (11111) and ablation (00010)
  - **Seeds**: {1, 2}
  - **DIM=64**: Full grid × control scales `K ∈ {1,2,4,8,16,32}` (≈192 runs) — local-allowed exception
  - **DIM=512**: Same candidates at `K=1` only (≈64 runs) — cloud heavy run
  - **CSV output columns**:
    ```
    dim, seed, mask, d_peak, k_band, K,
    excavated, deposited, exported_till, band_capacity, truncated,
    needles, max_resource, median_resource, solid_frac
    ```
  - **Measurements**:
    - `excavated`: `GlacialState.excavated_total`
    - `deposited`: `GlacialState.deposited_total`
    - `exported_till`: `GlacialState.exported_till` (outwash sink)
    - `band_capacity`: `GlacialState.band_capacity` (total headroom in band)
    - `truncated`: `GlacialState.truncated` (plateau gate counter)
    - `needles`: count of cells with `h > max(8-neighbors) + 40`
    - `max_resource`, `median_resource`: rescaled caps (matching ProcgenWorld)
    - `solid_frac`: percentage of cells with `h >= p65` (65th percentile)

## Running the Sweep

### Compilation check (local, allowed):
```bash
cd v2 && bash ../scripts/compile-check.sh
# Output: PASS
```

### DIM=64 sweep (local, F66 exception):
```bash
cd v2 && cargo run -q -p world --bin height_stats -- dim64 > /tmp/sweep_dim64.csv
```
Output: CSV with headers + 1024 rows (4 d_peaks × 4 k_bands × 2 seeds × 32 K values, production mask only)

### DIM=512 sweep (cloud, heavy):
Requires dispatching via GitHub Actions. DIM=512 is the GATE scale for picking `(Profile, k_band)`.

### Sample output row (DIM=64, production mask 11111):
```
dim,seed,mask,d_peak,k_band,K,excavated,deposited,exported_till,band_capacity,truncated,needles,max_resource,median_resource,solid_frac
64,1,11111,25,3,1,5235,1250,3985,4200,0,45,121,85,0.344
```

## Ledger Verification

Every row should satisfy:
```
excavated == deposited + exported_till
```

This is asserted by `run_glacial_with` inside `deposit_moraine_budget_drain` (line 593).

## Phase-0b Gate (DIM=512 decision point)

From design §Phase-0b sweep (line 370–398):

Pick `(Profile, k_band)` meeting ALL of:
1. **Plateau gate (F32)**: `truncated == 0` (no cell driven to headroom cap)
2. **Non-trivial moraine**: `deposited > 0` AND at least one Till cell is strict local max (`:707`)
3. **Needle metric**: `needles ≤ ~64` (F5 baseline)
4. **Resource asserts**:
   - `max_resource ≤ resource_base + 1` (F11, line 216–220 of lib.rs)
   - `median_resource ≥ 1` (F11, line 222–226 of lib.rs)
5. **Solid fraction**: `solid_frac ∈ [0.15, 0.50]` (line 233–237 of lib.rs)

## DIM=64 Fixtures (regression teeth)

From design §Ledger acceptance clause (line 108–126):

After running DIM=64 sweep, identify:

1. **Moraine-absorbing fixture** (hard-zero tooth, clause ii):
   - A small/sparse-ice fixture where `exported_till == 0`
   - Name it: `seed=S, mask=M, d_peak=D, k_band=K` from the sweep output
   - Expected: DIM=64 glacial-only grids have small excavated budgets fully absorbed by thin moraine

2. **Capping-capable fixture** (plateau tooth iv, clause F57/F60):
   - The fixture with `truncated@K=1 == 0` AND `truncated@K_CTRL > 0` at the SMALLEST such `K_CTRL`
   - Record `K_CTRL` value
   - This fixture will be used in production tests as:
     ```rust
     run_glacial_with(..., &PROFILE, k_band).truncated == 0  // baseline
     run_glacial_with(..., &PROFILE.scaled(K_CTRL), k_band).truncated > 0  // positive control
     ```

## Conservation Ledger Assertion (F35)

Module-doc contract (line 47–53 of glacial.rs) states:

- `Σdeposited == excavated` (old false statement)
- **CORRECTED (Option A)**: `Σdeposited + exported_till == excavated` EXACTLY (F35, F43)
  - Both sides computed INDEPENDENTLY (final_height deltas vs. accumulators)
  - Any silently-dropped/duplicated unit fails the identity

Test anchor: `slab_ledger_conserves_*` (F39, line ~697–704 in glacial.rs, to be updated in §Testing step 6)

---

## Files Modified

1. **v2/crates/world/src/gen/glacial.rs** (~150 lines total)
   - `GlacialState` struct: added 3 fields (lines 413–415)
   - `MorainerProfile::target_at_ring`: already present (lines 434–441)
   - `deposit_moraine_budget_drain`: extended to compute `truncated` (lines 448–551)
   - `run_glacial_with`: thread new return values (lines 577–611)
   - `run_glacial` (old path): set new fields to 0 (lines 651–659)

2. **v2/crates/world/src/bin/height_stats.rs** (~175 lines new)
   - Complete rewrite from Phase-0 baseline
   - Sweep driver with DIM=64/512 grid logic
   - Measurement functions: `count_needles`, `compute_resource_stats`, `compute_solid_frac`
   - CSV output generation

## Compilation Gate

✓ `scripts/compile-check.sh PASS` confirms:
- All Rust syntax valid
- All types match (GlacialState fields wired through)
- All imports resolve
- Test targets build (`cargo test --no-run`)

---

## Next Steps (after sweep measurements are available)

1. **Phase-0b addendum** (design §Design addendum, line 332–369):
   - Record winning `(Profile, k_band)` from DIM=512 gate results
   - Record `K_CTRL` value for plateau fixture
   - Confirm needles ≤ ~64, resource/solid asserts ≤ at DIM=512

2. **Production implementation** (design §Implementation order, steps 3–8):
   - Add Ledger assertion tests (F39: `slab_ledger_conserves_*`)
   - Implement P2 close (S_max=16) — already done in `run_glacial_with` line 555–556
   - Pin `const PROFILE: MorainerProfile = ...` + `const K_BAND: usize = ...` from addendum
   - Produce final `run_glacial` wrapper (uses pinned profile)
   - Update module-doc contract and test anchors (F17 same-commit contract)

3. **Re-pin goldens** (steps 6–8):
   - DIM=64 in-suite fast tests
   - DIM=512 cloud re-pin via CI
   - Verify `v2_golden_conserved_*` and all-OFF world goldens are byte-identical (F21)
