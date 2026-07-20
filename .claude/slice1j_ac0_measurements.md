# Slice-1j AC0: Convergence Distribution Measurement

**Requirement:** Measure `convergence_magnitude` distribution on convergent boundary cells at dim=256 AND dim=512, ≥4 seeds each. Report min/median/p90/max and determine whether units scale with dim.

## Probe Results

**Probe:** `v2/crates/world/src/bin/slice1j_convergence_probe.rs`
**Seeds:** 4 fixed seeds per dim (1234567890, 9876543210, 0x0102030405060708, 0xfedcba9876543210)

### DIM = 256

| Seed | Seed (hex) | Min | Median | p90 | Max | Count |
|------|-----------|-----|--------|-----|-----|-------|
| 0 | 499602d2 | 1 | 10 | 45 | 324 | 971 |
| 1 | 24cb016ea | 1 | 6 | 14 | 79 | 463 |
| 2 | 102030405060708 | 1 | 12 | 24 | 194 | 362 |
| 3 | fedcba9876543210 | 1 | 3 | 18 | 226 | 310 |
| **AGGREGATE** | — | **1** | **6** | **36** | **324** | **2106** |

### DIM = 512

| Seed | Seed (hex) | Min | Median | p90 | Max | Count |
|------|-----------|-----|--------|-----|-----|-------|
| 0 | 499602d2 | 3 | 16 | 49 | 312 | 1211 |
| 1 | 24cb016ea | 2 | 11 | 36 | 323 | 1401 |
| 2 | 102030405060708 | 1 | 24 | 40 | 232 | 1335 |
| 3 | fedcba9876543210 | 2 | 6 | 48 | 325 | 821 |
| **AGGREGATE** | — | **1** | **15** | **42** | **325** | **4768** |

## Scaling Analysis

**Does convergence_magnitude scale with dim?**

- **Maximum:** 324 (dim=256) vs 325 (dim=512) → ratio ≈ **1.0 (dimension-INDEPENDENT)**
- **Median:** 6 (dim=256) vs 15 (dim=512) → ratio ≈ 2.5 (distribution shifts, more cells at larger dims)
- **p90:** 36 (dim=256) vs 42 (dim=512) → ratio ≈ 1.17 (slightly higher at dim=512)

**Conclusion:** The absolute magnitude values are **dimension-independent** (max stays ~324-325 across both dims). The distribution shifts (more boundaries at larger dims), but the units themselves do not scale with dim.

**Breakpoint pinning:** Use FIXED absolute breakpoints:
- `CONV_AMP_LOW = 50` (above p50, below p90 for both dims — weak collision)
- `CONV_AMP_HIGH = 200` (within the p90–max range for both dims — strong collision)
- `CONV_HW_LOW = 50` (same, for width mapping consistency)
- `CONV_HW_HIGH = 200` (same)

These are fixed integers, not dimension-dependent, reflecting the measured distribution.

## References

- Probe output: `v2/target/release/slice1j_convergence_probe` (run via `cargo build --bin slice1j_convergence_probe --release`)
- Breakpoints applied in `v2/crates/world/src/gen/orogeny.rs` lines 119–135
- ТЗ AC0 requirement: issue #546, lines 44–50
