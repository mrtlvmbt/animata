# D-4 — Universal size-predation (the prevalence fix for the multicellularity transition)

## Problem (grounded this session, main @ 8ac6226)

The driver track (#42) reached a CONSOLIDATED landmark: all machinery built, three pre-declared
emergence experiments each an honest NULL, root diagnosed as **prevalence-limited**. Grounding of the
encounter model confirms and localizes the throttle:

- `stage_predation` (`v2/crates/sim-core/src/stages.rs:519-533`) splits a cell's entities by
  `combat_trait > 0 → predator, else → prey`; each predator's prey-pool is neighbours with strictly
  smaller `combat_trait`.
- **Within any predator-occupied cell, prevalence is already 100%** — every eligible prey is exposed.
  The throttle is NOT the encounter mechanic.
- The throttle is the predator **source**: founder `combat_trait = 0` (`genome.rs:553`); the mutation
  is a symmetric ±1 drift, gated by `mutation_rate`, clamped `[0,32]` (`genome.rs:645-652`). Predators
  arise rarely and drift back to 0 unless predation strongly pays. So the density of predator-occupied
  cells stays low → most prey never meet a predator → no population-wide selection gradient on body
  size, no matter how strong the refuge (round-3 result: even `bite_shift=0` gives WITH ≈ ablation ≈ 9%).

**Conclusion:** high prevalence requires a MODEL change (the combat-trait split is the throttle), not a
`--set`. Levers (a) predator-fraction and (b) encounter-density were rejected by grounding — neither
fixes the source. Chosen lever (user, this session): **(c) universal size-predation** — the Boraas
mechanism, and it REUSES the existing `resolve_encounter` + D-1 per-prey size-refuge path.

## Goal

Make predation **ubiquitous and size-selective by construction**: any entity may eat a strictly
smaller-BODIED neighbour in its cell; the D-1 size-refuge makes the bite shrink as prey body grows, so
a prey that grows past all local predators becomes uneatable (escape). This turns predation from a
rare-mutant event into an ever-present selective force on body size — the precondition the three NULLs
established was missing.

**This is a substrate/mechanic change, gated so every non-driver golden stays byte-identical.** It is
NOT a claim that the transition WILL emerge — that verdict remains the pre-declared
`driver_emergence_verdict` gate, unweakened.

## Design

### D-4a — the universal-size cell loop (sim-core)

Add an opt-in mode flag and a new cell-loop branch in `stage_predation`.

1. **Spec flag — the invalid state is UNREPRESENTABLE by type (F1 fix)**
   (`v2/crates/sim-core/src/predation.rs`): do NOT add a bare `bool` to `PredationSpec` (a
   `universal_size` next to `size_refuge: Option<_>` lets `true + None` be constructed, guarded only by
   a release-stripped `debug_assert` → the round-1/2 silent-fallback gap). Instead put the flag **inside
   `SizeRefugeSpec`** — which only exists when `size_refuge = Some(_)`:
   `SizeRefugeSpec { shift, refuge_k, universal: bool }`. Then "universal predation" cannot be requested
   without a refuge spec present — the `universal + no-refuge` state is unconstructable, no runtime guard
   needed. `universal: false` (every existing `Some` literal, plus `None`) = combat-trait split,
   byte-identical to P-2a/D-1. `universal: true` (only `driver_config`) = the new branch. `refuge_k = 0`
   with `universal: true` is a legitimate, representable "ubiquitous predation, no size-selection" mode
   (intentional, not silent). The one non-test setter is `driver_config`; test fixtures set
   `universal: false` (or `size_refuge: None`), keeping their asserted outcomes identical.

2. **Cell loop** (`stages.rs`): when `spec.size_refuge.map_or(false, |r| r.universal)` is `true`, take a
   **separate top-level branch evaluated BEFORE the combat-trait split** — NOT a sub-case of the existing
   `if spec.size_refuge.is_some()` gate (F1: that gate routes to the per-prey path, but the branch must
   OWN its resolution so there is no fall-through to the aggregate path). Because the `universal` flag
   lives inside `SizeRefugeSpec`, reaching this branch already proves `size_refuge = Some(_)` — no
   `debug_assert` / no unrepresentable-state guard needed. The branch calls `resolve_encounter` with
   `spec` directly (the refuge is always present; `refuge_k > 0` → size-scaled bite, `refuge_k = 0` →
   ubiquitous-but-not-size-selective, both intentional). For each cell:
   - Collect every entity in the cell as `(id_bits, Entity, body_size)`, `body_size = Σ
     Phenotype.graph.module_cell_count` (clamped ≥1) — same read the D-1 per-prey path already does.
   - Sort by `id_bits` (R14 single-writer order).
   - For each entity `E` in id order acting as PREDATOR: prey-pool = cell entities with **strictly
     smaller body** (`body < E.body`; ties → neither, strict `<` keeps it antisymmetric ⇒ no A↔B cycle,
     no self-predation). Resolve each prey via the EXISTING D-1 per-prey resolution (`resolve_encounter`
     with `size_refuge`, prey energy read **live** on each `q.get()` so a prey already drained by a
     lower-id predator this tick is seen post-drain — F5; drained in id order, death → despawn,
     conservation re-proven per prey by the resolver's invariant).
   - **Predator strength** feeds `pred_genome.size` (the `trait_factor` term). For D-4 the Boraas story
     is ESCAPE (fixed predator, prey escapes by size), so predator strength is **neutral**: set the
     trait term to 0 by driving it from `combat_trait` which stays 0 under this mode (founder never
     needs to mutate it), OR set `combat_trait_scale = 0` in `driver_config`. Bite is then governed by
     `bite_shift` × the prey's own refuge. (Knob deliberately left as a config value so the verdict
     experiment can later sweep predator-strength-scales-with-own-body as a DISTINCT hypothesis — out of
     scope for D-4a.)

3. **Determinism (R14):** no RNG, no float; id-order single-writer; strict `body <` is a total order on
   the antisymmetric relation (equal bodies never eat each other) ⇒ deterministic. **Conservation
   (R15):** unchanged — every drain goes through `resolve_encounter`'s `predator_gain + dissipated ==
   prey_loss ≤ prey.energy` invariant, applied once per prey, reading live prey energy. Byte-identical to
   the D-1 per-prey path, only the pool membership rule differs.

4. **Perf:** cell-local, worst case O(k²) per field-cell (k = per-cell population) — same order as
   today's predators×prey, now all entities are potential predators. Document the worst-case in a
   comment at the branch. **Regression guard already exists:** the D1a/F8 per-entity work-bound perf
   corridor (`cli/src/lib.rs:449`, sustained-population scenario) trips if predation introduces an
   O(N²) blowup — a pathological cell-occupancy is caught by CI, not left silent. (F4: Accepted
   tradeoff — see registry; no cap / density lever in D-4, that was rejected lever (b).)

5. **Body-size invariant — enforced at SETUP, not left silent (F2 fix):** `universal` predation is only
   meaningful when bodies actually VARY in size — a config with `morphogen = Some` + `evolve_body_size`.
   Otherwise `Phenotype.graph.module_cell_count` is empty → all bodies clamp to 1 → the strict `body <`
   pool is always empty → the branch resolves nothing. This degenerate case is BENIGN for conservation
   (no energy touched) but produces silently-wrong SCIENCE (a "predation" run with zero predation). So
   guard it where it is cheap and loud: a **one-time config-construction check in `build_sim`**
   (`cli/src/lib.rs`, runs once at setup, NOT per-tick) — `assert!(morphogen.is_some() &&
   evolve_body_size, "universal predation requires varying bodies …")` when any
   `size_refuge.universal == true`. This is a real `assert!` (present in release, affordable off the hot
   loop, mirroring `build_sim`'s existing config coercion) → a misconfiguration fails LOUDLY at startup,
   not as an invisible no-op. `driver_config` satisfies the precondition (`phase2_config`-derived,
   `g_dev=1` + `evolve_body_size`). Unit-test fixtures still fill `CellGraph.module_cell_count` with real
   varied sizes so the cell-loop tests are non-vacuous.

### D-4b — driver_config opt-in + verdict experiment (cli)

1. `driver_config` (`cli/src/lib.rs`): set `size_refuge = Some(SizeRefugeSpec { shift, refuge_k,
   universal: true })`; set predator strength neutral (`combat_trait_scale = 0`, documented). Keep
   `c_coord`, `evolve_body_size`, `g_dev = 1` as they are. This is the only behavioural config change;
   all other `Some` refuge specs (none shipped today) and every test fixture use `universal: false`.

2. **Verdict experiment (UNCHANGED gate):** rerun the pre-declared `driver-emergence` sim-run scenario /
   `driver_emergence_verdict` (`#[ignore]`, cli tests) — EMERGE_FLOOR=128/256, MARGIN=2×,
   SEED_MAJORITY=3/5, POP_FLOOR=10, three branches WITH / ablation-predators / channel-isolation.
   **DO NOT weaken any threshold to force a PASS.** Under universal predation the ablation branch =
   predation off (`econ.predation = None`), so WITH-vs-ablation now contrasts ubiquitous size-predation
   against none — the honest test of whether prevalence was the missing ingredient. Outcome is reported
   as-is (PASS = transition emerges; NULL = peel the next layer, name the new root).

3. **CARRY-FORWARD (V-5 F1):** add a `--set bite_shift` unit test to `d2_set_overrides` while this slice
   is touching `cli/src/lib.rs` (non-blocking mechanism check flagged at V-5).

## Determinism / golden checkpoints (must hold)

- Non-predation configs (`econ.predation = None`): `stage_predation` still early-returns → byte-identical
  → the 3 golden `state_checksum` locks + all corridors UNTOUCHED. **Confirm** `driver_config` has no
  golden lock (it is exercised only by the `#[ignore]` verdict harness) before merge — if any golden
  covers it, that is a STOP-and-report, single-writer rule (animata-sim skill §9).
- `SizeRefugeSpec` gains a field: every `SizeRefugeSpec { … }` construction site must set `universal`
  (compile-forced). Test fixtures in `predation.rs` and any P-2a tests → `universal: false` (or keep
  `size_refuge: None`), keeping their asserted outcomes identical. `PredationSpec`'s own shape is
  otherwise unchanged.
- The `size_refuge` combat-trait Q-format and refuge Q-format arithmetic are unchanged — only pool
  membership changes.

## Test plan (lands in the suite, runs in CI — not on the dev machine)

- sim-core unit: universal cell loop — a 3-entity cell with bodies {1, 2, 4} (fixtures fill
  `CellGraph.module_cell_count`, NOT `empty()` — F2) → the size-2 eats size-1, size-4 eats both smaller,
  size-1 eats nobody; conservation exact; id-order drain; equal-body pair → no predation. Determinism:
  two runs byte-identical.
- sim-core unit (F5): one prey + two larger-bodied predators in a cell → both runs byte-identical; the
  second (higher-id) predator sees the prey's POST-drain energy; prey death attributed to the first-id
  predator; ledger conservation exact across both drains.
- sim-core unit (F2): a `universal: true` cell of entities with EMPTY `CellGraph` (all body=1) →
  strict `body <` pool empty → zero predation (documents the cell-loop no-op boundary). Plus a
  cli/build-time test: a `universal: true` config WITHOUT morphogen/`evolve_body_size` → `build_sim`
  panics at setup (the F2 loud-guard), not a silent no-op.
- sim-core: `universal: false` path unchanged (existing P-2a/D-1 tests stay green, assert
  byte-identical outcomes).
- cli: `--set bite_shift` override unit (carry-forward).
- gate: push → `scripts/ci-report.sh` must exit 0 (test-x86 + golden-arm64 green) before merge.
- experiment: `scripts/sim-run.sh driver-emergence` (cloud) for the verdict — reported, not gated on a
  particular outcome.

## Scope boundaries (explicit non-goals for D-4)

- NOT sweeping predator-strength-scales-with-own-body (a distinct hypothesis; leave the knob as config).
- NOT adding a cell-population cap / spatial-density lever (b) — grounding rejected it as not fixing the
  source.
- NOT weakening the verdict thresholds.
- NOT touching render / UI (out of CI lane).

## Acceptance criteria

1. `stage_predation` gains a universal branch (top-level, before the combat-split, owns its resolution —
   no fall-through, F1); the flag lives inside `SizeRefugeSpec` (invalid `universal+no-refuge`
   unrepresentable, F1); `build_sim` asserts `universal ⇒ morphogen+evolve_body_size` at setup (F2); all
   non-driver refuge specs / fixtures `universal: false`; `driver_config` `universal: true` + neutral
   predator strength.
2. `ci-report.sh` exits 0 (goldens + corridors byte-identical; new unit tests green).
3. `driver-emergence` verdict rerun under universal predation, outcome reported honestly (PASS or a
   named NULL root), thresholds unweakened.
4. `--set bite_shift` unit test added (F3 confirmed: the `--set bite_shift` HANDLER exists from V-5,
   `cli/src/lib.rs:693`; only the unit test is missing — this is genuine deferred work, not redundant).

## Accepted operational risks (trade-offs)

- **F4 [tradeoff]** — universal predation is O(k²) per field-cell (all entities are potential predators
  vs smaller-bodied neighbours). Accepted because (a) k = per-field-cell occupancy is small under the
  driver world (`M_FIELD=1`, entities spread across the field grid), and (b) the existing D1a/F8
  per-entity work-bound perf corridor (`cli/src/lib.rs:449`) already trips on any O(N²) predation
  regression — the risk is CI-guarded, not silent. Rationale for no cap: adding a cell-population cap is
  rejected lever (b), out of D-4 scope.

## Critic resolution trace

**Round 1 (F1–F5):**
- **F1 [robustness]** — universal branch top-level, owns resolution, no fall-through (§D-4a.2).
- **F2 [robustness]** — body-varies invariant documented, fixtures fill `CellGraph`, no-op test (§D-4a.5).
- **F3 [style] → CONFIRMED valid, kept** — handler exists (V-5), unit test genuinely missing (acc §4).
- **F4 [tradeoff] → ACCEPTED** — registry above, CI-guarded by the existing perf corridor.
- **F5 [robustness] → FIXED (round-2 cleared)** — live post-drain read + two-predators/one-prey test.

**Round 2 (F1/F2 re-opened: `debug_assert` is release-stripped → silent misconfig):**
- **F1 [robustness] → FIXED** — the flag now lives INSIDE `SizeRefugeSpec`, so `universal + no-refuge`
  is **unrepresentable by type** — no runtime guard, no release gap (§D-4a.1/.2).
- **F2 [robustness] → FIXED** — the bodies-vary precondition is now a **real `assert!` at `build_sim`
  setup** (release-present, one-time, off the hot loop) → loud startup failure, not a silent no-op
  (§D-4a.5); + a build-time panic test.
- Round-2 confirmed no new blocking design defects (R14/R15/deferred-despawn sound).
