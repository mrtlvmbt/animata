# Plan — Terragen relief fix: attribute the pile (ablation) → fix glacial till geometry + ice_mask de-needle

## Pivot history
- **R1 critic** killed "remove clamps + global u8 normalize": monotone normalize can't remove piles/needles;
  u8 breaks `PHOTIC_H`; the extreme values come from glacial till DEPOSITION GEOMETRY, not clamping.
- **R2 critic** cleared F1–F6; found a 2nd non-glacial pile source (scarp/volcanic, F10), an already-lossy
  till ledger at 512 (F9), a defect-recreating percentile fallback (F8), the Till apron + resource asserts
  (F11), band direction/sink (F12), a too-heavy 512 CI test (F13).
- **R3 critic** cleared F8–F13; found the export ledger was a tautology with no regression tooth (F14), that
  "option-A monotone rescale" is the SAME datum-shifting transform constraint 3 forbids and F8 was rejected
  for (F15), an ablation confounded by option-A's flag gate (F16), and that `exported` breaks the existing
  DIM=64 ledger test + module-doc contract (F17).
- **Human decision on F15: option (A)** — **constraint 3 is law; NO datum-shifting rescale anywhere.**
  Option-A (the working-tree erosion min-max rescale) is REVERTED unconditionally. Scarp/volcanic overshoot
  is handled without shifting the datum (inert clamp booked in the ledger, or reduced additive magnitudes).
  No `PHOTIC_H` re-pin. Removing option-A also dissolves F16 (no rescale flag left to confound the ablation).

## Goal
The 512 diverse-relief map renders as believable relief — no flat ceiling plateau, no 1-cell needles —
with height in [0, hmax=200] on the production path (except the pre-existing erosion tail booked in the
risk section), mass conserved WITH the off-map outwash sink explicitly booked (regression teeth = the
independently-computed conservation identity + hard-zero on a moraine-absorbing fixture + a non-trivial
moraine ridge; Option A withdrew the ≤1% ε as physically wrong), and every ON-path `ProcgenWorld::new`
assert (solid-fraction AND both resource asserts) still satisfied.

## Non-negotiable constraints
1. **Integer-only in `gen/`** (`no_float_guard_gen.rs`) — no float; fixed-point/`i64`.
2. **Determinism / arch-independence** — pure functions; fixed iteration order; byte-identical x86/arm64.
3. **Height range stays [0, hmax=200]; NO u8; NO datum-shifting transform of any flavour** — forbids min→0,
   pin-at-hmax, AND min-max/percentile rescale (all shift the absolute-height datum and break `PHOTIC_H=200`
   / `HEIGHT_SCALE`). F8 and F15 are the same law. (Human-decided: F15=A.)
4. **Sim reads height** — `solid_level` (percentile), `is_solid`, gen-time biome/material/resource/climate
   classification, and `photic_atten` (absolute units). Preserve semantics or explicitly re-pin.
5. **Do NOT trip the ON-path hard asserts in `ProcgenWorld::new` (out of CI ⇒ they crash the user's 512
   run, not the build):** `world/src/lib.rs:233-237` `solid_frac ∈ [0.15,0.50]`; `:215-221`
   `max_resource <= resource_base+1`; `:222-226` `median_resource >= 1`. Checked against ALL THREE by
   measurement at 512 (F2, F11).
6. **Mass conservation is explicit with a REGRESSION TOOTH** — the off-map outwash sink is a booked ledger
   term, and the test asserts the accounting identity PLUS the single **Ledger acceptance clause** (§Ledger
   below: hard-zero on a capacity-fitting fixture + pre-registered ε on production); a future overflow makes
   the test fail honestly, not silently balance (F14, F25/F30). This is the ONE normative statement of the
   tooth — constraint 6, P1, and Testing all reference it, never restate a second contract.
7. **No local sim runs; no heavy per-CI-round tests** — goldens re-pin via CI (two-pass); prod-scale checks
   gated (F13).

## Phase 0 — MEASURE & ATTRIBUTE FIRST (on a clean, option-A-reverted baseline)
Revert option-A FIRST (step 1 below), then extend the throwaway `height_stats` bin at **dim=512** (+ 2nd
seed), BEFORE any production edit:
1. **Landform pile ablation (F10; clean now that no rescale flag confounds it, F16):** height max/p99/deciles
   for {all-OFF}, {volcanic+scarp-ON·glacial-OFF}, {glacial-ON·volcanic-OFF}, {both-ON}. Attributes the
   ceiling pile to additive scarp/volcanic vs glacial vs erosion-deposition. **Gate:** the scarp/volcanic
   overshoot handling (P1b) is chosen FROM this table.
2. **Erosion-deposition overshoot:** quantify `max > hmax` on {all-OFF} (R1 saw 237) AND `count(cells >
   hmax)` — decides whether the tail is a handful (leave, golden-neutral) or material (human decision, F21).
   Erosion is NOT clamped either way (F21/F22); this is the tail the glacial ledger carries.
2a. **Saturated-margin count (F23):** `count(incised == hmax ∩ ice-margin)` at DIM=64/256/512 — MEASURED
   (=0 everywhere). NOTE: this was the narrow-band criterion; the hard-zero tooth fixture is now chosen by
   CAPACITY (post-Route-1 excavated ≤ band capacity), NOT this count — see the Ledger acceptance clause + the
   Phase-0b sweep (F25/F30). Retained only as a measured datum.
3. **Per-margin-cell till deposit magnitude** (instrument `deposit_till`) — sizes the P1 geometry fix.
4. **`ice_mask` hole-size histogram** — enclosed non-ice component sizes → sets `S_max` for P2 (F5).
5. **Needle metric baseline** — `cell > max(8-neighbours) + 40` (F5).
6. **Resource + solid-fraction stats at 512** — `max_resource`, `median_resource`, `solid_frac` on the ON
   path, before AND after each candidate fix (F2, F11).
Output = numbers table appended here; a design addendum then fixes `k_band`/cap, `S_max`, the Till-apron
material decision, and the scarp/volcanic overshoot handling — all FROM the numbers.

## Conservation ledger with a regression tooth (F14, F17, F18, F21, F22)
**Do NOT clamp the erosion field at all** (R5 alternative). Adding a pre-glacial erosion clamp (the R4
idea) is rejected: it (a) IS the pin-at-hmax / flat-ceiling transform constraint 3 + the Goal forbid, just
renamed (F22), and (b) mutates the ALL-OFF path — which IS the sim world (`cli/src/lib.rs:993` builds
`ProcgenWorld` all-flags-false) — so it would move `v2_golden_conserved_*` (F21). Leaving erosion untouched
KEEPS the datum, keeps the sim/OFF path byte-identical (the ~237 tail is EXISTING behaviour the sim already
lives with — `photic_atten` already `clamp(h,0,200)`, `solid_level` is percentile), and adds no plateau.
- **[F43/F44] Deposit is BUDGET-DRAINED, not profile-fixed (one sink, one meaning — the critic's Alternative).**
  Start `remaining = excavated_total`. Walk band cells (post-close) in ascending distance-ring, ascending
  cell-index within a ring (F27 determinism). For each: `take = min(profile[idx], max(0, hmax−1 − incised[idx]),
  remaining)`; `final_height[idx] = incised[idx] + take`; `remaining −= take`. `profile` is the DESIGNED thin
  margin-peaked moraine (a small ridge, NOT `excavated/band_size` which was the plateau-maker). The `max(0,·)`
  guards the unclamped erosion tail (`incised > hmax−1` cells exist — F29).
- **[F47] Capping STILL physically happens** (the earlier "no cap-truncation" claim was FALSE): where
  `profile[idx] > headroom[idx]`, `take` clamps to headroom and the cell is driven TO `hmax−1` — a plateau
  cell — while the untaken mass stays in `remaining` (→ `exported_till`). Budget-drain renamed the overflow, it
  did NOT remove the physics. So the plateau needs its OWN explicit DEPOSIT-referenced tooth (below), not a
  safety-by-construction hand-wave. **[F56] `truncated = Σ over band cells with `headroom[idx] > 0` of
  `max(0, min(profile[idx], remaining@idx) − headroom[idx])`** — deposit mass the profile wanted beyond a
  cell's headroom. The `headroom>0` filter is MANDATORY: a band cell already in the unclamped erosion tail
  (`incised > hmax−1`, base max 237–241 — NOT clamped by design) has `headroom==0` and would truncate under ANY
  positive profile, however thin — counting those makes the `truncated==0` gate structurally unsatisfiable (no
  candidate passes → deadlock → Friday relax). A pre-existing over-hmax cell is an erosion fact, not a plateau.
- **[F33] DROP the `glacial.rs` ON-path clamp; tail cells keep `incised`.** Non-band and tail cells
  (`incised > hmax`, no till) keep their `incised` value — matching the untouched OFF/erosion path (F21/F22);
  no tail mass is laundered into the ledger. Re-adding the clamp is FORBIDDEN (it books the clamped tail as
  outwash — F33). (On production 11111, `coastal.rs` re-clamps `[0,hmax]` downstream, so the >hmax tail persists
  only on glacial-only masks.) Restate `post_glacial_height_stays_in_valid_range` (F38) → `≥0 ∧ ≤ max(hmax−1,
  max(incised))` in the SAME commit, else the drop trips it and the coder re-adds the clamp.
- **[F35/F44] Two INDEPENDENT counters, split by meaning (not fused):**
  `deposited_total = Σ over ALL cells (final_height − incised)` (measured from the height field, independent of
  the drain accumulator); `exported_till = remaining` (pure outwash, ≥0 by construction — Route 2). Identity
  `excavated == deposited_total + exported_till` then BITES: if any `final ≠ incised + take` (a stray clamp, a
  double-count) the height-delta sum ≠ `Σtake` and it fails.
- **The Ledger acceptance clause (THE tooth — single source of truth, referenced by constraint 6/P1/Testing);
  reframed for Option A (human 2026-07-13), which DROPS Route 1 and the ≤1% ε:**
  (i) `excavated_total == deposited_total + exported_till` ALWAYS, on every fixture (both sides computed
  INDEPENDENTLY — F35 — so this bites: a deposit routine that drops/duplicates units, or an off-edge term that
  doesn't balance, fails it);
  (ii) **hard `exported_till == 0` (`remaining == 0`) on a MORAINE-ABSORBING fixture** — a small/sparse-ice
  DIM=64 fixture whose `excavated` the thin moraine fully drains (well-defined under budget-drain — F43; the
  sweep names it; NOT keyed on `saturated_margin` — F25/F30);
  (iii) **the moraine is non-trivial** — `deposited_total > 0` AND `till_marks_at_least_one_local_height_maximum`
  (`:707`) holds on production. Replaces the (physically-wrong) ≤1% ε: catches a routine that exports EVERYTHING
  (no moraine) while allowing the large, correct outwash fraction. NO tight upper bound on `exported_till` — a
  large export (~80% on production; more on 00010) is legit glacial outwash (Route 2), booked in risk, NOT gated;
  (iv) **[F44/F47/F48/F51] plateau regression tooth — DEPOSIT-referenced, CAPPING-CAPABLE fixture, IN CI**
  (the critic's Alternative, verbatim): the 0b sweep emits `(excavated, band_capacity, Σprofile, truncated@×K)`
  per candidate over control scales `K ∈ {1,2,4,8,16,32}` (F60) (`band_capacity = Σ_band max(0, hmax−1 −
  incised[idx])` for context). **[F57/F60/F63] The CI fixture (a whole DIM=64 run — seed × mask, NOT a cell
  index; `truncated` is a per-run `GlacialState` counter) is the one with `truncated@×1 == 0 ∧
  truncated@×K_CTRL > 0` at the SMALLEST such `K_CTRL`** (recorded in the addendum) — selecting on that pair
  GUARANTEES the fixture is both clean at the pinned profile AND capping-capable at `K_CTRL` (the old `excavated
  > band_capacity` selector did NOT imply capping under a fixed profile — F57; a fixed `×4` may not cap at all
  for a thin profile with ample headroom — F60). Two in-suite asserts on it: `run_glacial_with(…,&PROFILE,…)
  .truncated == 0` (the tooth) AND `run_glacial_with(…,&PROFILE.scaled(K_CTRL),…).truncated > 0` (the positive
  control — non-vacuity PROVEN, not
  assumed). `truncated` is a `GlacialState` stat field (F51). DEPOSIT-referenced, no `final ∈ (hmax−10,hmax−1]`
  heuristic (F48/F32). The sweep confirms `truncated == 0` holds at DIM=512 too (the real plateau scale).
- **The erosion >hmax tail (F21/F22):** left untouched (Phase-0 (b): base max 237/241 — see risk section).
  Erosion is NOT clamped. The `max(0,·)` in `take` above is what keeps the tail from poisoning the ledger.
- **F17 same-commit contract:** introducing `exported_till`/budget-drain, computing `deposited_total` from the
  height-delta, and dropping the ON-path clamp all change `run_glacial`'s output, so the SAME commit MUST
  update ALL of the below. **[F45] Anchor by NAME, not line number** — the throwaway Phase-0/0b block (~490
  lines, inserted ~`glacial.rs:635`) has shifted every downstream line ref by ~+486; the coder greps each
  symbol/test by name (the `:NNN` tokens here are pre-throwaway-block and STALE):
  - the `run_glacial_with(…, profile, k_band)` seam + `run_glacial` pinned-const wrapper (F52–F58); the
    `Profile` type (per-ring deposit targets) + its `scaled(k)` operator; the budget-drain `take` loop
    (`final=incised+take`, `deposited_total = Σ(final−incised)`, `exported_till = remaining`, `truncated` per
    F56) — F33/F35/F43/F44/F55/F56;
  - `slab_ledger_conserves_*` (`glacial.rs:697-704`, NOT the Phase-0 `:517-524` measurement fn — F39) — assert
    the Ledger acceptance clause;
  - `post_glacial_height_stays_in_valid_range` (`:742-748`) — restated per F38 (`≥0 ∧ ≤ max(hmax−1,max(incised))`);
  - `glacial_off_leaves_non_ice_cells_untouched` (#416 orthogonality — F40) — restated to "non-ice cells
    OUTSIDE the k_band apron are byte-identical to input"; its in-code rationale ("deposit always ON an ice
    cell") is now FALSE and must be rewritten. **[F46] This AMENDS #416's ТЗ acceptance** ("outside the ice
    mask glacial is a no-op") — book it as an explicit ТЗ amendment in the PR body, not a silent test edit;
  - `till_marks_at_least_one_local_height_maximum` (F41) — still holds via the margin-peaked ridge +
    `Till iff applied>0`;
  - the U-trough / wall-drop test (`on_wall_total >= off_wall_total`, F42) + its doc ("till only deposits at
    the margin") — go FALSE (the outward band raises the non-ice neighbours the wall-drop measures against).
    **[F46] Concrete replacement (NOT "relax"):** measure `wall_drop` on the PRE-deposit `incised_height` (the
    U-trough is an incision property, and incision is untouched under Option A), OR exclude band/Till cells from
    the non-ice neighbour set. State the chosen form in the restated test;
  - **UNCHANGED (Option A — Route 1 dropped): incision, `K_ICE`, and `flatten_interior_components` are NOT
    touched** → `ice_incision_never_raises_a_cell` (`:692`), the single-signed module doc (`:43`,`:319-321`),
    and the floor-spread arm (`:911-913`) all stand as-is (no flatten knob → F34/F37 are moot);
  - module-doc contract `glacial.rs:48-53` ("Σdeposited == excavated" → "Σdeposited + exported_till ==
    excavated; exported_till==0 on the moraine-absorbing fixture; moraine non-trivial on production"),
    `deposit_till` doc (`:380-397`), and `GlacialState` doc/field — add `exported_till`, `truncated`, AND
    `band_capacity` as stat fields (F51/F57 — the tooth can't read a counter with no production home; stat-only,
    no height/material effect ⇒ goldens unaffected).
  **Standing rule (F40/F41/F42 root — do NOT list from memory):** before impl, `grep` glacial.rs for EVERY
  test/doc asserting WHERE/HOW till lands (deposit target, Till tagging, wall/spread, valid-range, non-ice
  orthogonality) and restate each to its new TRUE contract IN THE SAME COMMIT. A red CI test here = a MISSED
  contract to restate, NEVER a licence to relax/delete the assert.

## P1 — Fix glacial till deposition at the source  [F1, F11, F12, F24; Option A, human 2026-07-13]
Defect (Phase-0): `excavated` ≈ 3–4× the whole [0,hmax] range per margin cell — AREA-scale sediment that NO
geometry can place in-range without flattening the world (F24). **Option A mechanism** (Route 1 excavation-
reduction is DROPPED — troughs stay deep, `K_ICE`/`flatten`/incision UNCHANGED):
- **Thin capped margin-peaked MORAINE, deposited OUTWARD** — a designed thin ridge profile over non-ice cells
  within distance ≤ `k_band` of the ice margin (never inward — F12); margin-peaked → a moraine ridge (local
  max, `:707`); hard cap `applied ⇒ height ≤ hmax−1` (§Ledger) → no plateau by construction. Deterministic per
  the F27 spec (ascending-index seeds/rings, no HashSet).
- **[F52–F58] ONE full-pipeline seam, shared by sweep + production + tooth (the critic's Alternative — no
  drift, no missing budget, no pre-deposit recompute):** `pub(crate) fn run_glacial_with(seed, dim, hmax,
  height, profile: &Profile, k_band) -> GlacialState`, and `run_glacial` = the pinned-const wrapper
  (`run_glacial_with(…, &PROFILE, K_BAND)`). It drives the WHOLE pipeline — `ice_mask` → hole-fill (S_max=16) →
  `ice_incision_pass` → budget-drain deposit — so `excavated` is computed inside (no missing budget param, F55)
  and nothing downstream reconstructs the pre-deposit field by hand (no incision/hole-fill drift, F58).
  `GlacialState` carries `exported_till`, `truncated`, `band_capacity` (stat fields). The 0b sweep calls it over
  candidate `Profile`s (NOT the deleted `∝excavated` helper, F53); production is the pinned wrapper; the tooth's
  positive control is `run_glacial_with(…, &PROFILE.scaled(K_CTRL), …)` (a real call, not a fantasy const-scale,
  F52; `K_CTRL` is the single source of truth from Ledger clause (iv) / the 0b addendum — never a literal `×4`, F60).
  `Profile` (the per-ring deposit targets) + its `scaled(k)` operator are DEFINED in the impl and listed in F17.
- **Route 2 (off-map outwash) carries the rest** — `intended_offedge = excavated − Σ_band intended` is a LARGE
  (~80% on production) legit glacial-outwash sink, booked as `exported_till`. Acceptance is the single **Ledger
  acceptance clause** (identity + hard-zero on a moraine-absorbing fixture + non-trivial moraine; NO tight ε —
  that was the physically-wrong contract, dropped in Option A).
- **[F41] Till tagging = KEEP `material = Till iff applied>0` (`glacial.rs:447-449`) — Till marks the deposited
  BAND cells, not the ice margin.** (My earlier "Till on margin only" default was WRONG: margin cells now get
  ZERO deposit — the till went outward — so they are neither raised nor `applied>0`, breaking BOTH the
  `till_marks_at_least_one_local_height_maximum` invariant `:707-739` AND the `:447-449` rule.)
- **[F41] Deposit PROFILE is margin-peaked (a moraine ridge):** heavier near the ice margin, decaying outward,
  so the near-margin band cells form a strict local-max ridge — satisfying `:707` geomorphically (terminal/
  lateral moraines ARE ridges at the ice edge), not by accident.
- **Till-apron resource guard (F11) = MEASURED fallback, not the default:** a wide Till band widens `Till→(0,1)`
  barren (`caps.rs:226-234`) → can trip `median_resource` (already 20 @s2). Re-measure `max/median_resource`
  @512 for the chosen `(R,k_band)`; ONLY IF it trips, mitigate by tagging `Till` only where `applied ≥` a
  threshold (keeps the ridge cells Till, drops the thin outer skirt to its underlying material) — decided from
  the measured stats, still satisfying `:707`.

## P1b — DROPPED (Phase-0 ablation (a): scarp/volcanic do NOT pile)  [F10, F15=A, F19]
Phase-0 (a): mask 10100 has ceiling=0, max≤210 — scarp/volcanic LOWER relief, no additive pile. The ONLY
ceiling source is glacial. `fault_scarp`/`EDIFICE_PEAK_HEIGHT` are **NOT touched** (their goldens stay
byte-identical). No rescale (constraint 3), no accept-the-pile (F19) — there is no pile to handle here.

## P2 — Glacial ice_mask spatial coherence (de-needle)  [F5, F7]
After `ice_mask(...)`, before incision: a deterministic **hole-fill** sized by Phase-0's hole histogram —
fill enclosed non-ice components up to `S_max`. Reads an IMMUTABLE snapshot into a new buffer (no in-place
cascade — pure, order-independent, F7). Success: needle metric drops from ~1552 toward the ≤~64 baseline.

## Coupling P1↔P2 (F6)
Closing holes shrinks margin / grows interior ⇒ more `excavated` ⇒ more deposit. Implement P2 and P1
**together**; fix the moraine profile / `k_band` against the POST-CLOSE field in the Phase-0b sweep; RE-MEASURE
(needle + deposit-per-cell + resource + solid-fraction + the Ledger acceptance clause) on the post-close field.
The ledger makes coupled accounting exact regardless of the ratio.

## Implementation order
1. **Revert option-A** (working-tree erosion min-max rescale, `erosion.rs`) unconditionally — it violates
   constraint 3 (F15=A) and confounds the ablation (F16). Restore the original erosion path.
2. Phase-0 measurement/ablation bin → numbers table (DONE — see §Phase-0 RESULTS). **Gated.**
2b. **Phase-0b sweep (GATE, F31; Option A — profile × `k_band`, NO R knobs):** the sweep CALLS the ONE
   `run_glacial_with` seam (F53/F59 — NOT a `∝excavated` helper) over candidate profiles at DIM=512; pick the
   profile × `k_band` meeting: no-plateau (`truncated==0`, deposit-referenced, F32) ∧ needles ≤~64 ∧ resource/
   solid asserts ∧ non-trivial moraine + `:707` ridge (§Phase-0b sweep). Emit `(excavated, band_capacity,
   Σprofile, truncated@×1, truncated@×K)` per candidate over control scales `K ∈ {2,4,8,16,32}` (F60) + base
   `count(>hmax)` (F32); identify BOTH the DIM=64 moraine-absorbing fixture (hard-zero, ii) AND the DIM=64
   fixture with `truncated@×1==0 ∧ truncated@×K_CTRL>0` at the SMALLEST such `K_CTRL` (tooth iv — F57/F60).
   **No production edit before this.**
3. Ledger term + F17 same-commit updates (`slab_ledger_*` export arm asserting the Ledger acceptance clause,
   module doc, `GlacialState`). Apply the `max(0,·)` headroom guard (F29). DROP the `:445` ON-path clamp (F33).
   (Option A: NO Route-1 / `K_ICE` / `flatten` change — incision is untouched, troughs stay deep.)
4. P2 close (S_max=16) + P1 deposition — all inside the ONE `run_glacial_with` seam (budget-drain deposit,
   pinned `Profile`); `run_glacial` = the pinned wrapper (F59)
   + Route-2 off-map sink (together, F6), integer/deterministic per the F27 spec (ascending-index seeds/rings,
   no HashSet), snapshot semantics, outward-only, cap `≤hmax−1`. (Old step 5 "reduce additive magnitudes" is
   DELETED — P1b dropped, F31.)
6. Tests: **DIM=64 in-suite** (fast, F20): (a) ledger `excavated==deposited_total+exported_till`; (b) hard
   `exported_till==0` on the moraine-absorbing fixture; (c) **tooth (iv): `truncated==0` on the capping-capable
   fixture + positive control `run_glacial_with(…, &PROFILE.scaled(K_CTRL)).truncated > 0`** (F54/F60 — this
   MUST be in the test list, not just the sweep); (d) non-trivial moraine (`deposited>0` ∧ `:707`). dim=256/512 as `#[ignore]`/
   cloud (F13, F20). Existing determinism + updated `slab_ledger_*` + the grep-restated till-location tests pass.
   `compile-check.sh` PASS.
7. Re-measure the sweep gate metrics (deposit-per-cell, near-ceiling attributable to till, needles, exported)
   + the three ON-path asserts at 512.
8. Re-pin affected goldens via CI (two-pass). Because erosion is NOT clamped and the OFF path (glacial/
   volcanic default-off) is otherwise untouched, sim `v2_golden_conserved_*` + all-OFF world goldens
   (`caps.rs` golden fixture) stay byte-identical (F21 resolved); only landform-ON world-gen golden-vectors
   move. EXCEPTION: if Phase-0 step 2 forces a source fix of a material erosion tail, that moves sim goldens
   and is a human-surfaced decision (not folded here).
9. Delete ALL throwaway instrumentation before merge (F45), KEEPING `run_glacial_with` + its inner deposit
   helper (they ARE production now): delete the `height_stats` bin AND the Phase-0/0b block inside
   `src/gen/glacial.rs` — `phase0_glacial_stats`, the sweep DRIVER (grid loop + CSV) + its structs,
   `ice_incision_*_scaled`/`flatten_*_scaled` (dead under Option A), and the OLD `deposit_thin_band`
   (`intended ∝ excavated`, `excavated·w/weight_sum` — the PLATEAU-MAKER, F50; REPLACED by the seam's
   budget-drain deposit, not promoted). `grep` (SCOPED to `gen/glacial.rs` +
   `bin/height_stats.rs` — a bare `grep sweep` hits 25 unrelated files) for `phase0`/`_scaled`/`Phase0`/`sweep`/
   `deposit_thin_band` and remove; a leftover private fn reds clippy `dead_code`.

## Testing / verification
- `scripts/compile-check.sh` (pre-push).
- **DIM=64** in-suite (fast, F20): ledger `excavated==deposited_total+exported_till` ∧ hard **`exported_till==0`
  on the moraine-absorbing fixture** (step 2b, NOT saturated_margin — F25/F30) ∧ **tooth (iv) `truncated==0` on
  the capping-capable fixture + positive control `PROFILE.scaled(K_CTRL) ⇒ truncated>0`** (F54/F60 — `K_CTRL`
  by name from the 0b addendum, not a literal) ∧ non-trivial moraine
  (`deposited>0` ∧ `:707`); **dim=256/512** as
  `#[ignore]`/release-only or cloud `sim-run` (F13/F20; the non-trivial-moraine production arm — `deposited>0`
  ∧ `:707` ridge — lives here). Existing `*_is_deterministic_*` + UPDATED `slab_ledger_*` (F17). All reference
  the ONE Ledger acceptance clause.
- Needle metric + per-margin-deposit + decile histogram + resource/solid-fraction stats (throwaway bin)
  before/after at 512.
- The three ON-path asserts checked at 512 by measurement (out of CI, F2/F11).
- Render lane: `cargo build`/`clippy` + manual 512 run.

## Accepted operational risks (trade-offs)
- **[F28] base/OFF-path height reaches ~237–241 > hmax** (Phase-0 (b), seeds 1/2). This is PRE-EXISTING
  erosion (talus) behaviour the sim already lives with — the final field is unclamped, `photic_atten`
  clamps at read, `solid_level` is percentile, and the `v2_golden_conserved_*` / all-OFF world goldens
  already lock it. NOT introduced here; erosion is left untouched (F21/F22). severity: tradeoff.
- **[human-decided, Option A 2026-07-13] LARGE off-map outwash export is BY DESIGN** (`exported_till` ~80% on
  production 11111, more on 00010). Real glaciers carry most excavated sediment to the sea via meltwater
  (outwash); only a minority forms marginal moraines. So a large `exported_till` is faithful physics, NOT a
  bug — the ≤1% ε target was withdrawn. The regression teeth are the (independently-computed) conservation
  identity + hard-zero on a moraine-absorbing fixture + the non-trivial-moraine `:707` ridge. severity:
  tradeoff (physical). Supersedes the earlier "shallower troughs" trade — Route 1 is dropped, troughs stay deep.

## Out of scope
- u8/u16 output range; datum-shifting rescale of any flavour (F15=A); render height interpolation.
- Uplift-as-forcing geomorphic rewrite.
- Changing `WorldView::height` return type.
- Softening/removing any `ProcgenWorld::new` assert.

---

## Phase-0 RESULTS (measured 2026-07-12, clean option-A-reverted baseline, HMAX=200)
Instrument: throwaway `phase0_glacial_stats` (glacial.rs) + extended `height_stats` bin. compile-check PASS.
Flag mask = tect,aeol,volc,glac,coast (scarp armed by tect bit). Full tables in animata-pm
`reports/` (PHASE0_MEASUREMENTS + PHASE0_EXTENDED). Headlines:

**Height/ablation @512 (seed1 / seed2):**
| mask | max | ceiling(==hmax) | needles |
|---|---|---|---|
| 00000 base | 237 / 241 | 0 / 0 | 38 / 31 |
| 10100 scarp+volc | 210 / 198 | 0 / 0 | 29 / 20 |
| 00010 glacial | 200 / 200 | 22832 / 26807 | 554 / 636 |
| 10110 sc+vo+gl | 200 / 200 | 22690 / 21272 | 459 / 582 |
| 11111 all | 200 / 200 | 9118 / 8742 | 463 / 583 |

**Mass balance (excavated vs non-ice headroom `Σ(hmax−incised)` @512):**
| mask | excavated | incision% | flatten% | nonice_headroom | fits? |
|---|---|---|---|---|---|
| 00010 s1 | 19.6M | 68 | 32 | 9.0M | **NO (117%)** |
| 00010 s2 | 14.8M | 69 | 31 | 13.4M | **NO (111%)** |
| 11111/10110 s1 | 14.2M | 70 | 30 | 16.7M | YES (85%) |
| 11111/10110 s2 | 9.2M | 75 | 25 | 23.7M | YES (39%) |

**saturated_margin (incised==hmax ∩ margin) = 0 on ALL fixtures** (F23: hard `exported==0` tooth is
admissible). **S_max = 16** (`holes_le_16` > 90% of hole count, every grid/seed). **Resource/solid @512
ON path (11111):** `max_resource`=300, `median_resource`=220 (s1) / 20 (s2), `solid_frac`=0.356/0.358.

**Interpretation (the three questions):**
- **(a)** The ceiling is **glacial deposit truncation**, NOT additive scarp/volcanic. scarp/volc keep the
  field ≤210 with ceiling=0 (they LOWER mean relief); glacial `deposit_till` dumps `excavated/margin_len`
  ≈ 627–859 units on each of ~22–27k perimeter cells → every margin cell overflows hmax → clamp rim.
- **(b)** all-OFF base reaches 237–241 (> hmax) with ceiling=0 — the final field is NOT clamped; this is
  EXISTING sim behaviour (goldens lock it, `photic_atten` clamps at read, `solid_level` is percentile).
  Leave erosion untouched (F21/F22 confirmed).
- **(c)** [superseded by Phase-0b + Option A] Total non-ice headroom nominally exceeds excavated on 11111, but
  the 0b sweep showed the DEPOSITABLE band capacity (≤5.3M @k=8) is ≪ excavated (14.7M) — filling to headroom
  would re-create the plateau (F24). Option A therefore deposits a thin moraine + exports the ~80% remainder as
  outwash; the ceiling is removed by the thin cap, not by fitting all the mass in-range.

## Design addendum (post-critic F24–F42; mechanism = Option A, human 2026-07-13)
Trail: v1's "wide BFS fill-to-headroom" was a `bug` (F24 — pin-at-hmax, plateau scaled up). Root (critic +
math + Phase-0/0b): `excavated` ≈ 3–4× the whole [0,hmax] range per margin cell AND `excavated` (14.7M @512
11111) ≫ in-range band capacity (≤5.3M @k=8) — the mass is physically un-placeable in-range by ANY geometry.
v2 tried Route 1 (reduce excavation) to force it in-range; the 0b sweep proved that needs near-total incision
elimination (troughs gone) to hit ≤1% export. **Option A (human): DON'T force it in-range.** Deposit a thin
moraine, export the rest as legit outwash — because that IS the geomorphology (glaciers carry most sediment to
sea). Mechanism:

1. **P1b (scarp/volcanic additive reduction) = DROPPED.** Ablation (a): no additive pile (10100 ceiling=0,
   max≤210). Only ceiling source is glacial. `fault_scarp`/`EDIFICE_PEAK_HEIGHT` untouched.
2. **Route 1 (excavation reduction) = DROPPED (Option A).** `K_ICE`, `flatten_interior_components`, and the
   whole incision pass are UNCHANGED → troughs stay deep, incision goldens/invariants untouched (F34/F37 moot).
3. **P1 deposition = thin capped margin-peaked MORAINE, outward.** A DESIGNED thin ridge profile (peak deposit
   magnitude + outward decay) over non-ice cells within distance ≤ `k_band` of the margin (immutable snapshot;
   never inward — F12); `applied ⇒ height ≤ hmax−1` (cap). A thin capped ridge has no cell near hmax ⇒ no
   plateau by construction (F24 closed). Profile + `k_band` fixed by the 0b sweep.
4. **Route 2 — off-map outwash carries the LARGE remainder.** `intended_offedge = excavated − Σ_band intended`
   (~80% on production) is booked as `exported_till` — faithful glacial outwash, not a bug. No tight upper
   gate (the ≤1% ε is withdrawn — it required destroying the troughs).
5. **[F27] Determinism SPEC:** margin seeds enqueued in ascending cell-index order; band by distance ring;
   within a ring, ascending cell-index; deposit integer/associative; NO HashSet/HashMap. Hard requirement or
   `v2_golden_conserved_*`/world-vectors go arch-flaky.
6. **[F25/F30] Ledger tooth (see §Ledger acceptance clause):** hard `exported_till==0` on a MORAINE-ABSORBING
   DIM=64 fixture (tiny-ice: `excavated ≤ Σ moraine-profile capacity`, so the thin moraine places all of it —
   named by the 0b sweep; NOT `saturated_margin`). Identity `excavated == deposited + exported_till` (computed
   independently, F35) holds on ALL fixtures.
7. **[F26→Option A] ε WITHDRAWN.** No `exported_till ≤ 1%` gate — a large export is correct physics. Replaced
   by the non-trivial-moraine smoke test (`deposited>0` ∧ `:707` ridge) so a "deposit nothing, export all"
   regression still fails.
8. **[F41] Till tagging = `material = Till iff applied>0`** (`:447-449`, kept) → Till = the deposited moraine
   band cells (raised ridge ⇒ `:707` local-max holds). F11 resource guard is a MEASURED fallback (tag `Till`
   only where `applied ≥` threshold) ONLY IF `median_resource` trips @512 — not the default.
9. **P2 hole-fill S_max = 16.** Re-measure POST-close (F6): filling ≤16-cell holes adds ice (raises excavated);
   size the moraine profile + `k_band` against the POST-close field.
10. **[F33/F38] Erosion tail untouched; DROP the `:445` ON-path clamp** (tail cells keep `incised`, matching
    OFF path); restate `post_glacial_height_stays_in_valid_range` to `≥0 ∧ ≤ max(hmax−1, max(incised))`.

## Phase-0b sweep (GATES production impl — Option A: moraine profile × `k_band`, NO R knobs)
CALL the ONE `run_glacial_with(seed, dim, hmax, height, &Profile, k_band) -> GlacialState` seam (F53/F59 — the
SAME code production ships, so numbers transfer; it drives ice_mask→hole-fill→incision→deposit, no hand-rebuilt
pre-deposit field, F55/F58) over a grid: thin moraine `Profile` (peak `d_peak` ∈ e.g. {15,25,40,60} × outward
decay) × `k_band` ∈ {2,3,5,8}. `K_ICE`/flatten/incision UNCHANGED. **[F64/F66] Run plan (the critic's
Alternative):**
- **DIM=64:** the FULL candidate grid × `K ∈ {1,2,4,8,16,32}` (≈192 runs) — cheap; booked as a DELIBERATE
  local-run exception to CLAUDE.md's "any new check in the cloud" (F66: DIM=64 worldgen is trivial). This is
  where `K_CTRL` + the tooth-(iv) fixture are read off the winning profile's rows.
- **DIM=512:** the SAME candidate grid at `K=1` only (≈64 runs) — a **cloud `sim-run`** (heavy). THIS is the
  gate scale that picks `(Profile, k_band)` on the plateau gate `truncated==0`.
Measure, on production (11111) AND ablation (00010), 2 seeds, from `GlacialState`'s
`(height, deposited_total, exported_till, truncated, band_capacity)`:
- `excavated_total`; `deposited_total`; `exported_till` (outwash remainder — reported, NOT gated); confirm
  identity `excavated == deposited_total + exported_till`;
- **plateau gate (deposit-referenced, F32/F47/F48):** `truncated == 0` (no cell driven to its cap). (Absolute
  `ceiling(==hmax)`/`final∈(hmax−10,hmax−1]` are confounded by the erosion tail — also emit base `count(>hmax)`
  for context, F32.)
- **non-trivial moraine:** `deposited_total > 0` ∧ at least one Till cell is a strict local max (`:707`);
- needles after S_max=16 close (target ≤~64); `max/median_resource` (≥1), `solid_frac` (∈[0.15,0.50]).
Pick `(Profile, k_band)` meeting: `truncated==0` ∧ non-trivial moraine ∧ needles ≤~64 ∧ resource/solid asserts
— AT DIM=512 (the gate scale). Numbers → addendum table; then name the DIM=64 moraine-absorbing fixture
(`exported_till==0`) for tooth (ii) AND the DIM=64 fixture with `truncated@×1==0 ∧ truncated@×K_CTRL>0` at the
smallest such `K_CTRL` (F57/F60) for tooth (iv) + record `K_CTRL` for its `Profile.scaled(K_CTRL)` positive
control. **[F65] If NO (seed×mask, K≤32) pair satisfies the fixture predicate for the winning profile:** extend
the K ladder OR re-pick `(Profile, k_band)` — NEVER drop the positive control or the tooth (that is the
forbidden "relax the assert" move). (F61 style: `profile==headroom` exactly drives a cell to `hmax−1` with `truncated==0` — an
exact-equality cap the deposit-referenced tooth doesn't see; it cannot cluster into a plateau, so note it,
don't gate on it.)

## Phase-0b RESULTS (measured 2026-07-13, `run_glacial_with` seam, HMAX=200) — GATE PASS
Pinned production consts: **`d_peak = 40`, `k_band = 5`** (margin-peaked moraine; a ~40-unit crest ridge just
outside the ice, decaying over 5 rings — visible but not dominant; render-tunable). `S_max = 16`.
**DIM=512 gate (production 11111, both seeds, ALL candidate profiles):** `truncated == 0` everywhere (plateau
impossible); `needles == 0` (was 459–583 — S_max=16 hole-fill); `median_resource = 9` (≥1); `solid_frac ≈ 0.35`
(∈[0.15,0.50]); identity `excavated == deposited + exported_till` exact (0 mismatch). `exported_till` ≈ 91–99%
(large off-map outwash — Option A by design). For d=40/k=5: deposited ≈ 624k (s1) / 706k (s2) = 4–7% moraine.
**Tooth fixtures (confirmed by measurement):**
- **(ii) hard-zero:** `dim=64, seed=2, mask=00010` → excavated=788, deposited=788, `exported_till==0`, truncated=0.
- **(iv) plateau + positive control:** `dim=64, seed=1, mask=11111`, **`K_CTRL=4`** → `truncated@×1==0 ∧
  truncated@×4==10003 (>0)` (also seed2 ×4=10104). Non-vacuous.
- identity holds on every fixture. Design fully validated → Stage 2 (wire `run_glacial`→seam, restate tests, CI re-pin).
