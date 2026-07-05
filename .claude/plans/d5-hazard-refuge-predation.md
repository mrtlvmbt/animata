# D-5 — Hazard-refuge predation (conditional size-defense — a FAITHFUL multicellularity driver)

## Problem (grounded: D-4 verdict + deep-research, main @ 3c597d7)

D-4 universal size-predation made the multicellularity transition EMERGE (WITH ~95% vs ablation ~9%),
but the cloud verdict + adversarial research diagnose it as an **artifact**: channel-isolation (refuge
off) stays ~90% ≈ WITH, body pins to the ~95% ceiling. Size wins UNCONDITIONALLY because universal
`body<`-predation rewards size on TWO count-based (eligibility) channels — **offense** (#smaller
neighbours you may eat grows with body) AND **defense** (#larger neighbours that may eat you shrinks with
body). The size-refuge (bite-scaling) is only a third, non-load-bearing channel.

**Faithful-driver criterion** (deep-research, `RnD/scratchpad/multicellularity-drivers-research-findings.md`,
verified: Ratcliffe 2012 settling +34%/−10%, Lowery 2017 division-of-labor persistence; 23 single-driver
claims refuted): a non-artifact driver must satisfy ALL of — (a) advantage REVERSIBLE when the pressure is
removed; (b) intermediate phenotypes PERSIST (no sweep to a pure-size ceiling); (c) measurable COST under
relaxed selection; (d) the fitness valley is crossed ONLY in a specific ecological context, not universally
via `size > coordination-cost`. **D-4 fails (b) and (d).**

**The core tension D-4 exposed:** the pre-D-4 model had CONDITIONALITY (size = defense-refuge only) but no
PREVALENCE (predators were rare `combat>0` mutants → predation too rare to select). D-4 bought prevalence by
making everyone a size-predator, but lost conditionality. **D-5 must deliver BOTH.**

## Goal

Replace the `body<` predation-ELIGIBILITY (the offense+defense count channels) with a **prevalent background
predation hazard that body size mitigates ONLY through the refuge (defense-only)** — the Boraas selective
story abstracted. Size then helps survival ONLY while the hazard is present (conditional + reversible), pays
a coordination cost always (`c_coord`), and — depending on the hazard↔cost balance — settles at a stable
INTERMEDIATE body size (a valley/plateau) rather than a ceiling. This is the experiment: does a
conditional, prevalent, defense-only pressure produce a faithful transition where D-4's unconditional one
produced an artifact?

## Design

### D-5a — the hazard-refuge mechanic (sim-core)

A new predation MODE where predation is an implicit external predator applying a per-entity per-tick energy
drain, refuge-attenuated by the entity's OWN body size. No entity-vs-entity eligibility; no offense.

1. **Spec** (`v2/crates/sim-core/src/predation.rs`): add a mode enum
   **`PredationMode { CombatSplit, Universal, Hazard }`** on `PredationSpec` (F2: an ENUM, NOT a
   `universal`/`hazard` bool-pair — a bool-pair guarded only by a release-stripped `debug_assert` lets
   both-true be constructed silently; the enum makes mode-exclusivity type-guaranteed, mirroring the D-4
   type-over-runtime-guard lesson). Migrate D-4's `SizeRefugeSpec.universal` into this enum
   (`Universal`); `CombatSplit` = the legacy path; `None` predation is unchanged. Hazard mode adds
   `base_hazard: i64` (the un-refuged per-tick drain) on `PredationSpec`. REUSE the existing refuge
   Q-format (`resolve_encounter`'s `x << shift / (2^shift + k·body)`) to attenuate `base_hazard` by body
   size — NO new refuge math, same monotone-decreasing curve. **F3: `base_hazard` is defensively capped at
   `VALUE_MAX` (1e6, the `resolve_encounter` bound) before the `<< shift` so `(base_hazard << shift)`
   cannot overflow the widened accumulator — same guard the refuge path already relies on; the `--set`
   handler rejects out-of-range values.**

2. **Stage** (`v2/crates/sim-core/src/stages.rs`, `stage_predation`): when the mode is `hazard`, a NEW
   top-level branch (before combat-split AND before the D-4 universal branch), owning its resolution
   (no fall-through). For EACH entity, in entity-id order (R14):
   - `drain = refuge_attenuate(base_hazard, own_body_size)` — bigger body → smaller drain. NO neighbours
     read → per-entity independent → order-independence is trivial (stronger determinism than D-4).
   - `drain = drain.min(energy)`; `energy -= drain`; route the removed energy to dissipation
     (`ledger.dissipated`) — the implicit predator consumes it (conservation-exact, R15). Death when
     `energy ≤ 0` → despawn (existing routing).
   - `base_hazard = 0` ⇒ inert (no drain) = the ablation control, byte-identical to no-predation.

3. **Determinism (R14):** no RNG, no float; per-entity drain in id order, no cross-entity dependency.
   **Conservation (R15):** every drain → `ledger.dissipated`, `drain ≤ energy`, exact integer. Simpler and
   MORE robust than D-4 (no multi-predator-on-one-prey ordering subtlety — F5 there doesn't arise here).

4. **The faithful properties (F1 — the interior-optimum reasoning made explicit):**
   (a) `base_hazard=0` → size benefit gone → reversible; (c) `c_coord` cost persists; (d) size only helps
   while the hazard is present.
   **(b) intermediate persistence — why the mechanic CAN produce it, and how the sweep locates it:** the
   size-benefit is the DRAIN SAVED, `saved(b) = base_hazard − attenuate(base_hazard,b) = base_hazard·k·b /
   (2^shift + k·b)` — INCREASING but CONCAVE/SATURATING (marginal benefit `d/db` falls with b). The cost
   `c_coord·b` is LINEAR (constant marginal cost). Concave-saturating benefit minus linear cost ⇒ marginal
   net crosses zero exactly once ⇒ **an interior optimum `b*` EXISTS** — provided `base_hazard` is in the
   band where (i) the initial marginal benefit at b=1→2 EXCEEDS `c_coord` (else floor: never worth growing)
   and (ii) saturation bites BEFORE `MAX_CELLS` (else ceiling, like D-4). **The `base_hazard` sweep is the
   explicit LOCATOR of that band.** This is NOT asserted-by-construction; it is the experiment.
   **PRE-DECLARED HONEST NULL:** if NO `base_hazard` in the sweep yields a stable interior `b*` (only floor
   or ceiling across the whole range), that is a reported NULL — "monotone-refuge + linear-cost cannot
   produce a faithful intermediate; needs a concave-refuge / convex-cost / multi-hazard re-architecture" —
   NOT a threshold to weaken. (The critic's alternatives — log-refuge, diminishing-returns c_coord,
   multiple hazard types — are the named follow-ups if this NULL lands.)

### D-5b — driver_config + verdict harness (cli)

1. `driver_config` (`cli/src/lib.rs`): switch predation to `hazard` mode with a starting `base_hazard`
   (+ keep `refuge_k`, `c_coord`, `evolve_body_size`, `g_dev=1`). Pick VIABILITY-first defaults (population
   must survive the corridor — mirror the existing `d2_driver_config_viable` gate); the intermediate-vs-
   ceiling MEASUREMENT is the verdict's job, not tuned into the config.

2. **`--set base_hazard`** whitelisted (like `bite_shift`/`refuge_k`) so the verdict can sweep the
   hazard↔cost balance. Validate range; require `hazard` mode configured.

3. **Verdict harness — the channel-isolation control is now MEANINGFUL again** (this is the research-informed
   redesign that #282's FLAG deferred). Under hazard-refuge, refuge is the ONLY size-benefit channel, so:
   - **WITH** = hazard on + refuge on (refuge_k=128) → size selected.
   - **ablation** = hazard off (`base_hazard=0`) → no size selection (~unicellular).
   - **channel-isolation** = hazard on + refuge_k=0 → the drain is a CONSTANT `base_hazard/2^shift`,
     body-INDEPENDENT (F5: this is a constant drain, NOT zero — ablation is the zero-drain arm). Because
     size confers no survival benefit under a body-independent drain, there is no selection for size, so the
     multicellular_frac OUTCOME collapses to ≈ ablation even though the drain magnitude differs. **This is
     the clean specificity test D-4 broke and #282 flagged** — under hazard mode refuge is the ONLY
     size-benefit channel, so refuge_k=0 genuinely isolates it.
   Sweep `base_hazard` (as V-5 swept bite_shift). Thresholds UNCHANGED (EMERGE_FLOOR=128/256, MARGIN=2×,
   SEED_MAJORITY=3/5, POP_FLOOR=10). **DO NOT weaken.** ADD an intermediate-persistence readout (mean body
   size sits BELOW `MAX_CELLS` ceiling and above unicellular — the (b) criterion), reported informationally.
   Optionally keep a D-4 `universal` arm as the labelled ARTIFACT-CONTRAST (expected to pin the ceiling).

## Determinism / golden checkpoints (must hold)

- Non-predation configs (`predation=None`): `stage_predation` early-returns → byte-identical → the 3 golden
  `state_checksum` locks + corridors UNTOUCHED.
- `driver_config` changes behaviour again → **re-pin `GOLDEN_CONSERVED_DRIVER`** (arm64, PM single-writer,
  the established `pasted by the PM` procedure — dump `run_conserved_hashes(driver_config, 384)` on matched
  arch, verify against the golden-arm64 CI job; confirm x86 invariants (R15) stay green). `state_hash` still
  excludes predation, so only this driver conserved golden moves.
- Spec gains fields: every `SizeRefugeSpec`/`PredationSpec` construction site compile-forced to set them
  (fixtures → hazard off / universal off, keeping their asserted outcomes identical).

## Test plan (in the suite, runs in CI)

- sim-core unit: hazard drain — an entity with body b1<b2, same base_hazard → the larger body loses LESS
  energy (refuge attenuates); `base_hazard=0` → zero drain (ablation byte-identical); two runs byte-identical.
- sim-core unit: conservation — Σenergy_after + dissipated == Σenergy_before (exact); death at energy≤0 →
  despawn, ledger exact.
- sim-core unit: determinism — per-entity id-order, no neighbour read; two runs byte-identical.
- sim-core: `universal`/`combat-split` paths unchanged (existing D-1/D-4 tests byte-identical).
- cli: `--set base_hazard` override unit; a `hazard`-mode `driver_config` builds + is viable.
- cli/build-time: hazard mode + refuge preconditions guarded (reuse the D-4 F2 setup-assert pattern where
  applicable).
- gate: push → `ci-report.sh` exit 0.
- experiment: `sim-run.sh driver-emergence` (cloud) — reported honestly (does a hazard regime PASS all three
  UNWEAKENED conditions in ≥3/5 seeds, WITH the chan-iso control now dropping to ≈ablation? and does mean
  body sit at a stable INTERMEDIATE?). PASS or a NAMED NULL.

## Scope boundaries (non-goals)

- NOT settling / spatial-gravity selection (a separate, bigger slice — research's cleanest but costlier).
- NOT environmental variance / seasonality (co-factor, separate slice).
- NOT removing the D-4 `universal` mode (keep it as the artifact-contrast arm).
- NOT weakening verdict thresholds. NOT touching render/UI.

## Acceptance criteria

1. `stage_predation` gains a `hazard` mode (top-level branch, owns resolution, per-entity refuge-attenuated
   drain → dissipation, no offense/eligibility, no neighbour read); mode-exclusivity guarded.
2. `driver_config` uses `hazard`; `--set base_hazard` whitelisted; viability gate green.
3. Verdict harness channel-isolation restored to a meaningful specificity test (chan-iso = hazard on,
   refuge_k=0); intermediate-persistence readout added; thresholds UNWEAKENED.
4. `ci-report.sh` exit 0; non-predation goldens byte-identical; `GOLDEN_CONSERVED_DRIVER` re-pinned (PM).
5. `driver-emergence` verdict rerun, outcome reported honestly (PASS or named NULL); if PASS, the effect is
   channel-specific (WITH ≥ 2× chan-iso) AND body sits at a **stable intermediate** — the faithful
   signature. **F6 — "stable intermediate" is metrically defined:** the late-window (ticks [7000,8000])
   mean body size sits STRICTLY between unicellular (`> 1` cell, above ablation) and the ceiling (`<
   MAX_CELLS`, e.g. ≤ 0.9·MAX_CELLS) AND is drift-flat (|mean(second-half) − mean(first-half) of the
   window| below a small pre-declared epsilon). Reported informationally alongside the pass/fail (not a
   silently-added blocking threshold).

## Round-1 critic resolution trace

- **F1 [robustness] → FIXED** — interior-optimum reasoning made explicit (concave-saturating drain-saved −
  linear cost ⇒ single interior `b*` for a `base_hazard` band; the sweep locates it) + a PRE-DECLARED honest
  NULL if no band exists (§D-5a.4). The mechanic is shown to HAVE the ingredients, not assumed to.
- **F2 [robustness] → FIXED** — `PredationMode` ENUM (type-guaranteed exclusivity), not a bool-pair +
  release-stripped `debug_assert` (§D-5a.1).
- **F3 [robustness] → FIXED** — `base_hazard` capped at `VALUE_MAX` before `<< shift`; `--set` rejects
  out-of-range (§D-5a.1).
- **F5 [robustness] → FIXED** — chan-iso narrative corrected: constant body-independent drain (≠ zero); the
  OUTCOME collapses to ≈ablation because size gains no benefit (§D-5b.3).
- **F6 [style] → FIXED** — "stable intermediate" metrically defined (acceptance §5).
- **Ablation byte-identity clarified:** `base_hazard=0` in hazard mode = zero drain = zero state change (a
  verdict arm); non-predation GOLDENS stay byte-identical via `predation=None`'s early return — a distinct
  claim from the arm.
