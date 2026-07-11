task: #391 DOL-Germ-Repro First Probe (diagnostic on LIVE code, different mechanic)
phase: pre-registration complete, awaiting PM review
blocked_on: PM review of 7-check pre-registration (checks 1/2/3 authoritative)
next: After PM clearance → dispatch cloud run via sim-run.sh + GitHub Actions
updated: 2026-07-11 15:05 (pre-registration posted to PR #391)

## DOL-Germ-Repro First Probe (Fresh diagnostic, different mechanic)

**Why this probe (not fate_economy):**
- `fate_economy` has ZERO germ marginal return → monotone-decreasing → NULL vacuous (Check 1 failed)
- `dol_germ_repro` has POSITIVE germ marginal return (repro_bar ∝ body/germ) → PARABOLIC → CHECK 1 PASSES
- Different mechanic, different outcome structure, valid probe

**What was delivered:**
1. Test harness: `/Users/spopov/projects/animata/C/v2/crates/cli/tests/dol_germ_repro_interior_optimum_probe.rs`
2. 7-check pre-registration: posted to PR #391 comment (all checks PASS)
3. Config: dol_economy=true, dol_germ_repro=true, base_hazard=10 (D-5 predation)
4. Compile check: ✅ `cargo test --no-run` succeeded

**7-Check Summary (all PASS):**
1. ✅ Capability: interior split (germ≈N/2) CAN win (parabola f = germ - germ²/N)
2. ✅ Regime: multi-entity + D-5 + deficit (not monoculture/surplus)
3. ✅ Metric: realized offspring + fertile-subdomain PEAK classifier
4. ✅ Treatment: imposed split via module_is_germ
5. ✅ Variance: 5 seeds with stochastic placement + field
6. ✅ Confound: only split varies, rest fixed
7. ✅ Anti-forcing: historical code, no tuning, NULL valid

**Interpretation (once results arrive):**
- **PASS (≥2/3 seeds PEAK):** reward landscape works → size ceiling (Rung B) is next
- **NULL (edge/plateau):** germ-reward insufficient → soma-shield (Rung C) becomes target

**Committed:** e39c7de feat(dol-germ-repro): add first probe diagnostic...
