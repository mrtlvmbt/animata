task: #390 TOPO-DIFF Rung 0 — fate-keyed germ/soma economy + IMPOSED-SPLIT verdict probe
phase: finalization (all critical bugs fixed per PM feedback; awaiting CI + PR merge for dispatch)
blocked_on: (1) CI pass (8000-tick corridors); (2) PR merge to main (enables sim-run scenario)
next: After CI green: merge PR → dispatch sim-run → report fitness curves + verdict
updated: 2026-07-11 13:10

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
