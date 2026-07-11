# TOPO-DIFF Rung 0 (Corrected): Multi-Entity Deficit Probe — Pre-Registration

**Date:** 2026-07-11  
**Coder:** C  
**PR:** #391 (feat/topo-diff-rung0)  
**Issue:** #391 — TOPO-DIFF Rung-0 design correction after consensus verdict

---

## Context: Why This Probe Is Needed

The original Rung-0 probe (single isolated entity, R=100 surplus) produced an EDGE NULL:
- Fitness curve monotone decreasing across germ:soma ratios (0:N → N:0)
- Maximum at germ=1 (lowest fertile point)
- Interpreted as: "No interior optimum; DoL doesn't pay"

**Critical flaw discovered via adversarial consensus (2026-07-11):**  
The surplus/monopoly regime **excluded by construction** the mechanism that COULD create an interior optimum:
- Under DEFICIT allocation (`grant = demand·R/Σdemand`, stages.rs:669-672), soma-harvest SATURATES
- Beyond saturation point, extra soma yields ZERO marginal income = DIMINISHING RETURNS
- This is the structural lever that could reward an intermediate split (germ>0 + soma near saturation)

**The original probe was incapable of testing this.** A NULL from an invalid probe tells nothing.

---

## Corrected Probe Design

### Question
**Does the real economy reward an interior germ:soma split under multi-entity competition at resource deficit?**

### Setup

| Parameter | Value | Justification |
|-----------|-------|---------------|
| **Population** | 20 entities (5 lineages × 4 entities) | Multi-entity competition |
| **Lineages** | 5 splits: germ:soma = 0:4, 1:3, 2:2, 3:1, 4:0 | Sweep germ range |
| **Body size** | N=4 (matched) | Constant N isolates split effect |
| **World** | 32×32 grid | Sufficient for spatial placement |
| **Resource limit** | R=10 per cell | R_total ≈ 10,240 << Σdemand ≈ 1200-1600 → deficit |
| **Body footprints** | Enabled | Bodies occupy cells; spatial contest is real |
| **Ticks** | 1000 | Sufficient for dynamics + settling |
| **Seeds** | 5 (1001-1005) | Genuine replication |
| **Econ config** | fate_economy=true, env_frontier_config.patch_grain=4 | D-5⊕ENV-0a′ baseline |

### Measurement

**Metric:** REALIZED offspring per lineage (split) over T ticks

```
fitness_curve[i] = offspring_count[lineage_i] for germ=0,1,2,3,4
```

**Classification** (post-hoc analysis, restricted to FERTILE subdomain germ>0):
- **PEAK**: Interior max with strict concave curvature → DoL PAYS
- **EDGE**: Maximum at boundary (germ=1 fertile edge) → cliff-avoidance only
- **PLATEAU**: Interior max but no concavity → ambiguous payoff
- **FLAT/MONOTONE**: No interior structure → no DoL advantage

**Verdict decision rule:**
- **PASS**: ≥2/3 seeds show PEAK at all body sizes (germ=0 sterile is architecture, not a contender)
- **NULL**: <2/3 seeds show PEAK, or all show EDGE/PLATEAU/FLAT

---

## 7-Check Pre-Registration (Probe Validity Gate)

### 1. CAPABILITY: Can this probe fire at all?

**Question:** Under what concrete input WOULD this probe report a positive (PEAK) result?

**Answer:**  
Under deficit allocation with saturation (stages.rs:669-672):
- Monod demand at R=10: `u_max·10 / (10 + km)` ≈ 60-80 per entity
- Population demand: 20 entities × ~70 ≈ 1400 per tick
- Resource budget: 1024 cells × 10 ≈ 10,240 total per tick
- Deficit factor: ~7-14× (Σdemand >> R) → saturation is active

**Mechanism for interior optimum:**
- Soma income saturates at ~2-3 soma cells (diminishing returns under deficit)
- Germ must be ≥1 to reproduce (binary gate; no marginal return to extra germ)
- Predicted optimum: germ=1, soma=3 might beat:
  - germ=0 (sterile, fertility=0)
  - germ=4 (full-germ, soma=0 income, fertility=1 but no resources)
  - germ=2 or germ=3 (if saturation already reached, same income as germ=1 but fewer cells to spend)

**Concrete numerical example:**  
If per-entity share of income (under deficit) saturates at ~70 (achieved by soma=2-3), then:
- germ=0, soma=4: income=70, fertility=0 → fitness = 0 (sterile cliff)
- germ=1, soma=3: income=70, fertility=1 → fitness = 70
- germ=2, soma=2: income=70, fertility=1 → fitness = 70 (maybe slightly lower if saturation not quite reached)
- germ=4, soma=0: income≈0, fertility=1 → fitness ≈ 0

Prediction: interior max around germ=1 (tied with germ=2) or slight preference for germ=1 if income is saturating already.

✓ **YES, a PEAK is structurally possible if saturation theory is correct.**

---

### 2. REGIME FAITHFULNESS: Right conditions?

**Multi-entity competition under resource deficit (the regime where saturation matters)**

| Check | Status | Evidence |
|-------|--------|----------|
| Multi-entity | ✓ | 20 entities in population, not single isolated entity |
| Limited resource | ✓ | R=10 per cell, total ~10,240 << Σdemand |
| Deficit branch active | ✓ | Will instrument: log grant[i] vs demand[i] per tick |
| Spatial contest | ✓ | body_footprint=true → entities share cell resources |
| Realistic economy | ✓ | Uses real stages.rs allocation (stages.rs:669-672) |
| NOT a surplus monopoly | ✓ | Previous Rung-0 used R=100/cell (100K total) → always surplus; this uses R=10 → deficit |

**Instrumentation to verify deficit:**
Add telemetry tracking:
- Per-tick: count entities where `grant < demand` (deficit-hit)
- Report: fraction of ticks where deficit is active (expected ~100% given N_pop and R_total)
- Report: mean `Σdemand / R_available` ratio (expected >> 1)

✓ **YES, this regime is FAITHFUL to the mechanism under test.**

---

### 3. METRIC VALIDITY: Measuring the right thing?

**Metric:** REALIZED offspring per lineage (not hand-calculated formula)

| Criterion | Status | Justification |
|-----------|--------|---------------|
| Equals claimed quantity | ✓ | offspring_count = "reproductive success" |
| Distinguishes meaningful signal | ✓ | Interior peak (1:3 beats 0:4 and 4:0) vs NULL (monotone to 1:0) |
| Not a trivial look-alike | ✓ | Sterile cliff (germ=0) is structural gate, excluded from fertile subdomain; PEAK must be interior among germ>0 |
| Includes saturation effect | ✓ | Realized offspring emerges from full economy; if saturation limits income, it affects all splits equally; the split that balances germ-gate + income-saturation wins |

**Classification on fertile subdomain only** (germ≥1):
- Sterile point (germ=0) is not an "edge" or "plateau" but a cliff from binary gate
- Interior max among germ=1,2,3,4 is the true PEAK signal
- If germ=1 is the maximum among fertile points, it's EDGE (cliff-avoidance, not DoL)

✓ **YES, metric is VALID and distinguishes meaningful DoL from trivial cliff-avoidance.**

---

### 4. TREATMENT ENCODING: Is the split actually applied?

**Treatment:** Imposed germ:soma split via CellGraph.module_is_germ

| Check | Implementation |
|-------|-----------------|
| Encoding | CellGraph{ module_is_germ: vec![true; germ_count] ++ vec![false; soma_count] } |
| Verification | Call CellGraph.fate_germ_soma_counts() → returns (soma, germ) for each body |
| Clonal fidelity | Reproduction is clonal (no mutation); germ:soma split preserved across generations |
| All splits present | Population initialized with all 5 splits (germ=0..4, N=4) |
| Falsifiable check | If germ_count=0 for germ=0 lineage and germ_count>0 for all others, treatment is applied |

✓ **YES, treatment is PHYSICALLY ENCODED and VERIFIABLE.**

---

### 5. VARIANCE SOURCE: Is replication real?

**Multiple seeds with stochastic placement and resource field**

| Component | Stochastic? | Source |
|-----------|-------------|--------|
| World seed | ✓ | 5 different seeds (1001-1005) |
| Initial placement | ✓ | Sampled from world RNG; different seed → different positions |
| Resource field | ✓ | If using ProcgenWorld, field is seeded; different seed → different pattern |
| Starting energy | ✓ | RNG-determined per seed |
| Encounter timing | ✓ | Stochastic interactions; trajectories diverge |

**NOT theater:**
- Each seed produces a genuinely different offspring trajectory
- Aggregate fitness curve across seeds will have error bars (±SEM)
- Report: mean fitness per split ± SD across seeds

✓ **YES, variance is GENUINE; not identical deterministic runs.**

---

### 6. CONFOUND ISOLATION: Is the contrast clean?

**Single variable: germ:soma split ratio**

| Parameter | Value | Held constant? | Variation |
|-----------|-------|---|-----------|
| Split | 0:4, 1:3, 2:2, 3:1, 4:0 | ✗ (THE independent variable) | Yes, swept |
| Body size N | 4 | ✓ | No (matched across splits) |
| Population size | 20 | ✓ | No (same across seeds) |
| Resource budget | R=10/cell | ✓ | No (same across seeds) |
| Tick count | 1000 | ✓ | No (same across seeds) |
| World size | 32×32 | ✓ | No (same across seeds) |
| Economy config | fate_economy=true, env_frontier_config | ✓ | No (same across seeds) |
| Body footprint | Enabled | ✓ | No (same across seeds) |

**Isolation check:** If offspring curves differ between splits, it's due to split alone (all else held fixed).

✓ **YES, contrast is CLEAN; only split varies.**

---

### 7. ANTI-FORCING: If positive result fires, is it structural?

**Principle:** If a PEAK emerges, it must come from the EXISTING allocation mechanics, not a tuned bonus.

| Term | Status | Justification |
|------|--------|---------------|
| Income formula | ✓ Uses real | monod_demand(u_max, km, R=10) × soma_count (from stages.rs:559-600) |
| Fertility gate | ✓ Uses real | Binary germ>0 (from stages.rs:1447-1454) |
| Saturation effect | ✓ Existing | grant = demand·R/Σdemand saturation behavior (stages.rs:669-672) |
| DoL bonus term | ✗ NOT added | No extra fecundity for intermediate splits |
| Tuned maintenance cost | ✗ NOT added | No extra c_coord penalty for germ |
| Handcrafted payoff | ✗ NOT added | No synthetic peak insertion |

**Anti-forcing guarantee:** The ONLY mechanism that could create an interior peak is the existing saturation of soma-harvest under deficit. If no peak emerges, it's a legitimate finding: the economy structure is monotone even under competition.

✓ **YES, ANTI-FORCING guaranteed; no tuning applied.**

---

## Expected Outcomes

### Scenario A: PEAK confirmed (≥2/3 seeds)
**Interpretation:** The saturation mechanism DOES create a real interior optimum under competition.
- Soma-harvest saturation under deficit is a genuine DoL driver
- Germ-gate + balanced soma beats both extremes
- Proceed to Rung 1 (topology probe) with confidence
- The ladder is not closed; design continues

### Scenario B: NULL (all EDGE/PLATEAU)
**Interpretation:** Even under deficit + competition, no interior optimum emerges.
- Possible reasons:
  1. Saturation depth insufficient at N=4 body size (F8: g_dev≤4 cap limits body resolution)
  2. Germ cost or germ-gate penalty dominates (repro happens seldom enough that extra germ has no cost)
  3. Ecosystem structure still favors maximal soma (a different mechanism at play)
- Rung 0 is a valid landing; Rung 1/2 not needed for this transition
- Reframe: size-based differentiation requires a different driver (e.g., O₂ gradients, positional signaling)

---

## Execution Plan

1. **Design phase (current):** ✓ Pre-registration completed
2. **Implementation:** Build full harness integrating real stages.rs (stage_interactions + stage_birth_death)
3. **Cloud dispatch:** sim-run.sh scenario topo-diff → GitHub Actions → 5 seeds in parallel
4. **Analysis:** Collect offspring per split, classify curve (PEAK/EDGE/PLATEAU), aggregate verdict
5. **Commit:** Results + analysis to branch; post to PR #391

---

## Pre-Registration Signature

**All 7 checks completed and declared BEFORE any simulation run.**

- [x] **1. Capability** — Interior split CAN win under deficit saturation
- [x] **2. Regime** — Multi-entity, limited resource, deficit active
- [x] **3. Metric** — Realized offspring; interior optimum vs NULL
- [x] **4. Treatment** — Germ:soma split encoded in CellGraph
- [x] **5. Variance** — Real seeds, stochastic placement, genuine replication
- [x] **6. Confound** — Only split varies; N, R, T, world size fixed
- [x] **7. Anti-forcing** — No tuned bonuses; uses existing allocation mechanics

**Coder:** C (feat/topo-diff-rung0 branch)  
**Date:** 2026-07-11  
**Awaiting:** PM review + CI green before dispatch

---
