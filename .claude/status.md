task: #391 TOPO-DIFF Rung-0 corrected probe — multi-entity deficit (consensus verdict fix 2026-07-11)
phase: design-complete, pre-registration ready (NOT dispatched yet)
blocked_on: PM review + approval of 7-check pre-registration validity gates (all 7 passed)
next: Once PM clears: dispatch to cloud via sim-run.sh scenario topo-diff (GitHub Actions, 5 seeds)
updated: 2026-07-11 14:45

## Design & Pre-Registration Complete

**Previous Rung-0:** Degenerate (single entity, R=100 surplus) → NULL by construction
**Corrected Rung-0:** Multi-entity deficit (20 clones, R=10/cell, footprints enabled)

**Deliverables (committed to branch):**
✅ topo_diff_rung0_multientity_deficit_probe.rs (test scaffold, compiles green)
✅ topo-rung0-deficit-probe-preregistration.md (detailed 7-check analysis)
✅ PR #391 comment (pre-registration summary, all 7 checks)
✅ Commit: e322ff5

## All Critical Bugs Fixed

### Code-Critic Round 1 (F1–F5 inventory)
✅ F1: .fate_germ_soma_counts() → replaced with iterator pattern (114c05b)
✅ F2: Format string braces → fixed with format!() macro (114c05b)
✅ F3: Dead variable per_cell_soma → removed (114c05b)
✅ F5: module_is_germ=vec![false] → now encodes split (germ=true, soma=false) (114c05b)
✅ Measurement validation: Added docstring proving formula matches real stages (114c05b)
✅ Robustness check: Added fate_economy=FALSE arm for state-leakage detection (114c05b)

### PM Critical Feedback (Classifier + Seed Theater)
✅ FALSE PASS BUG: Classifier was accepting sterile cliff (germ=0→1) as rising shoulder
   - Root: empty windows(2) → vacuous .all() = true
   - Fix: Analyze fertile subdomain only (fertility>0 points)
   - germ=1 correctly classified as EDGE (fertile domain boundary) → NULL (e8de6d7)

✅ SEED THEATER: All 3 seeds produce identical curves (no RNG in measure_fitness)
   - Root: measure_fitness has zero randomness
   - Fix: Simplified to single deterministic run (TEST_SEED)
   - Honest reporting: one curve per body size, actual classification (e8de6d7)

### Current Test Status
- ✅ Compiles to release without errors
- ⏳ CI running (test-x86 + golden-arm64 corridors, run #29153866658)
- Ready to dispatch once: CI passes + PR merges to main
