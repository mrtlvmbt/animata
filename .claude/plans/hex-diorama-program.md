# Plan — "Hex diorama": v2 terragen relief/material diversity + hex-prism render look-pack

## User goal (verbatim intent)
Hex-voxel terrain that looks like the reference (Irene Checo hexworld): toy-diorama hex prisms,
smooth-yet-diverse relief, visually distinct BARE materials (rock, sand, soils of different
composition) — NO water, NO vegetation in the render. Performant generator AND renderer.
Program = research done (3 fan-out reports, 2026-07-13); this plan slices the work for coders
A/B (Sonnet). PM orchestrates only.

## Grounded state (from code recon, anchors verified)
- **v2 render ALREADY does flat-top hex prisms**: `v2/crates/render/src/hex.rs` (odd-q offset,
  HEX_SIZE=1.0, HEIGHT_SCALE=0.2), `terrain.rs` (top fan 4 tris + cliff quads only where neighbor
  strictly lower, per-chunk Mesh ≤65k u16 indices, adaptive `rows_per_chunk`), `camera.rs`
  (iso-ortho, 60° yaw steps, Gribb-Hartmann frustum + AABB cull), `biome_palette.rs` (9-material
  palette, directional shade AMBIENT=0.3/DIFFUSE=0.7, cliff ×0.6, hypsometric ColorMode toggle).
  NO AO, NO bevel, NO per-column variation, NO water suppression, NO screenshot/bench facility.
- **v2 worldgen**: integer-only (`no_float_guard_gen`), hmax=200, pipeline
  height→material→drainage→erode(incision+talus, 8 iters)→volcanic→glacial→aeolian→coastal→
  de_needle(NEEDLE_MARGIN=30)→classify_and_caps. Landform flags default-OFF (conserved goldens =
  OFF path). Materials: Air/Sand/Permafrost/Soil/Bedrock/Basalt/Tuff/Till/Water; assignment is
  landform-first, then biome surface, then depth (SOIL_DEPTH=4). Moisture + slope computed in
  classify but NOT exposed. ~100ms @512.
- **Known defect (diagnosed 2026-07-13, memory + measurements)**: thermal `talus_step` runs ONLY
  inside early `erode`; glacial/aeolian/coastal land AFTER → their spikes never relax; blunt
  `de_needle_pass` leaves residual +30 single-column "picket fence". v1 (global final talus) is
  smooth by construction.
- **v1 = reference viewer only.** v1→v2 dump bridge (ATDMP1, `v1_map_dump` → `--v1-dump`) is
  built, uncommitted on this branch; commit it as-is for comparison tooling (no productization).
- **Web research verdict**: chunk-merged meshes + frustum cull is the right architecture in
  macroquad (no instancing in 0.4); ~30 verts/cell worst-case is fine at 512² on Apple Silicon
  (3–5M tri budget safe); AO = bake per-vertex from neighbor heights; bevel = small chamfer ring
  (geometry) preferred over normal tricks at this art scale; per-column value jitter from
  hash(q,r); hypsometric value × material hue palette.

## Decisions
- **D1 — consolidate on v2** (gen + render). v1 stays as visual reference via the bridge; no new
  v1 work. (v2 gen is ~70× faster and materially richer; v2 render already hex.)
- **D2 — integration branch = `render-r12-terragen-preview`** (PR #428, marked NOT-for-master).
  Slices PR into it; master merge only at program end with the user.
- **D3 — two lanes, two gates**: worldgen lane (coder A) = full CI + golden discipline
  (conserved goldens byte-identical; landform-ON vectors re-pin two-pass via CI). Render lane
  (coder B) = out of CI: `compile-check.sh` (render workspace) + clippy + in-app screenshot
  evidence (render-lane merge policy, user 2026-07-03).
- **D4 — bare-relief is a RENDER concern.** Worldgen keeps Water/biomes (sim needs them). The
  render gets a `--bare` mode: Water cells tinted as wet-sand/dry-bed, no vegetation drawn (v2
  render draws none anyway). No worldgen change for this.
- **D5 — single-writer**: only coder A touches `v2/crates/world/**`; only coder B touches
  `v2/crates/render/**`. No shared files except sim-core trait IF W-10c (below) is approved.

## Slices

### Lane W (coder A — worldgen, CI-gated)

**W-9 — final-surface thermal relaxation (kills the picket fence).**
> **AMENDMENT (PM, 2026-07-14, post-sweep escalation — authoritative over the donor/gate text
> below; full text = issue #432 comment):** the first sweep proved uniform diffusion structurally
> cannot pass the step gate without grinding legitimate crests (till p10 retention 8%, dune 31%
> at the only gate-passing configs — a moraine crest IS a ~40-amplitude local max). Donor rule
> is now SPIKE-SELECTIVE: a cell donates ONLY if `h_old[v] − second_max(D8) > SPIKE_MARGIN`
> (needle: second-max = ground ⇒ donates; ridge crest: second-max = adjacent ridge cell ⇒ never
> donates). Pair transfer/scaling/determinism unchanged. Gate: zero cells with
> `h − second_max(D8) > MAX_SPIKE_FINAL = 12` ∧ needles==0 (replaces the max-step form; a legit
> ridge passes by construction). Sweep: SPIKE_MARGIN {8,12,16} × iters {2,4,8}. Twin-needle
> mutual shielding = booked residual (same pre-existing blindness as de_needle).
After coastal, before classify: N iterations of a NEW `talus_step_final(dim, height, threshold)`
over the final height field — integer, gather-not-scatter, mass-conserving WITHIN the scaled
loop (the final un-scale floors ≤1 unit/cell — see below; do NOT bill the unscaled output as
Σ-conserved). **Exact rule
(pair-wise Jacobi diffusion on a FIXED-POINT scaled copy — bounded on BOTH sides,
quantization-free):** scale once into `hs = h · 64` (i64 working copy). Per iteration, read
frame `hs_old`; for each ordered pair (v,u) of D8 neighbors with
`hs_old[v] − hs_old[u] > thr_s` (`thr_s = threshold·64`), transfer
`t(v,u) = (hs_old[v] − hs_old[u] − thr_s) / 2 / 8` (integer division; the divisor is the
CONSTANT D8 degree 8, NOT a per-cell k_lower — constant divisor bounds the receiver: a pit
with 8 higher neighbors gains ≤ (max_drop − thr)/2, i.e. it can rise at most halfway to its
lowest higher neighbor, NEVER invert into a spike; donor-side loss is likewise ≤ half its
excess). Both sides compute t(v,u) from the SAME `hs_old` — donor subtracts Σt, receiver adds
Σt ⇒ `Σhs` invariant per iteration BY CONSTRUCTION. The ×64 scale removes the integer
deadzone (at unit scale `(drop−thr)/16 = 0` for every drop < thr+16 — the pass would silently
no-op on exactly the needles it targets). After N iterations un-scale once: `h = hs / 64`
(floor, deterministic; ≤1 unit/cell quantization documented). **Invariant test SHIPS IN THE
SLICE as RANGE CONTRACTION, not just Σ:** per iteration `Σhs` invariant ∧
`max(hs_new) ≤ max(hs_old)` ∧ `min(hs_new) ≥ min(hs_old)` — the max/min clauses catch both a
receiver-side inversion (max would grow) AND a silent no-op deadzone (max unchanged across all
iterations on a needle fixture fails a companion "max strictly decreases on the needle
fixture" assert). POST-UNSCALE the slice asserts the properties it actually needs — needle
count and local-max step bound on the output field — NOT Σ (the floor loss makes an unscaled
Σ test wrong by design). It takes NO drainage input, so there is no
stale-`downstream` dependency (the post-coastal surface has no recomputed drainage; the old
`talus_step(dim, height, downstream)` at erosion.rs:214 would move mass along PRE-landform
receivers — zero-sum but not smoothing — and its single-receiver dump makes needles WALK
instead of dissipating). The old `talus_step` is NOT touched ⇒ erode path byte-identical.
REPOSE_THRESHOLD_FINAL calibrated for the final surface (landform apices sit 30–100 over
neighbors; the erode-loop's threshold=0 does NOT transfer — it would flatten
dunes/moraines/edifices into mush).
- Phase-0 (measure first, throwaway bin): slope histogram (|Δh| to D8 neighbor) at 512 all-ON,
  2 seeds; landform amplitude signatures BEFORE the pass. **The amplitude metric is PRODUCTION
  CODE, not bin code, and it is MASK-KEYED (heights alone cannot identify landforms):**
  `pub fn landform_amplitudes(dim, heights: &[i64], masks: &LandformMasks) -> AmplitudeReport`
  in `world::gen`. **ONE scoring primitive for all three masks — no flood-fill, no per-mask
  special case, no set-max escape:**
  1. **Freeze a per-landform CREST LIST on the PRE-talus field:** crests = strict local maxima
     (in-grid D8) within the mask, with `pre_amplitude(c) = h_pre[c] − median(h_pre of c's
     in-grid D8 ring) ≥ AMPLITUDE_FLOOR`. The crest CELLS are frozen — the post-pass call
     evaluates the SAME cells (a post-derived crest set would survivor-bias toward tall crests
     and re-open the set-max escape; same freeze rule as the masks).
  2. **Amplitude at a crest = `h[c] − median(D8 ring of c)`**, evaluated pre and post at the
     same c. **All arithmetic i64 fixed-point:** `retention_pct(c) = 100·post/pre` — the fn
     lives under `world/src/gen/**` where `no_float_guard_gen` FAILS CI on any f32/f64, and a
     bare i64 `post/pre` truncates every retention <100% to 0. Comparisons as
     `100·post ≥ 60·pre` style, never a float.
  3. **Score = p10 of the crest retention_pct list per landform.**
  4. **Floors validated as a Phase-0 PRECONDITION — on crest COUNT, not mere nonemptiness:**
     each mask must yield ≥16 qualifying crests @512 (≥4 @64) at `AMPLITUDE_FLOOR` (start =
     MAX_LOCAL_STEP_FINAL; recalibrate from the Phase-0 numbers BEFORE the sweep if under).
     A p10 over n≈3 is min-in-disguise — the noise-hostage this spec rejects. Till is the
     known risk: the ring-constant deposit (`target_at_ring`, glacial.rs:543) makes a ridge
     LINE whose along-ridge neighbors tie ⇒ strict maxima can be scarce on flat incised
     stretches. **Fallback order PRE-DECIDED (if the count precondition fails):** (1)
     non-strict maxima (`≥` all D8 neighbors, plateau-tolerant), (2) ring radius 2 for the
     median. Never invented at sweep time. Empty mask / empty crest list = a documented
     `Option/Err` caller contract (the OFF path has all-empty masks — a panic there is a
     latent bomb), never a silent 0/1.
  Till gets NO component treatment — the moraine is a distance-ring BAND (glacial.rs:539-559),
  one big component per valley, but it is margin-PEAKED ⇒ many crests, and the crest primitive
  scores each; dunes likewise (the `sand_depth>0` sheet is one component but many crests).
  Masks are NOT mutually exclusive (the volcanic>Till>sand priority chain applies to material,
  not to the raw masks; coastal submerged cells stay in) — fine for a retention ratio, do NOT
  invent exclusion.
  **AMPLITUDE IS A REPORT-AND-DECIDE METRIC, NOT A SHIPPING ASSERT — WITH ITS OWN TRIGGER:**
  the sweep REPORTS per-landform p10 retention for every config; **if the PICKED config's p10
  retention < 60 on ANY landform → STOP, human decision** (this trigger is independent of
  grid-emptiness — a config can pass needles+step while mushing moraines to 20%, and "grid
  not empty" must not launder that). Procedurally, "human-reviewed at pick" = the retention
  table is IN the PR body and the PR does not merge until PM/user explicitly signs the
  numbers off (a checklist item, done-gate visible). The per-crest p10 metric itself ships
  NO assert (ten critic rounds showed every frozen crest assert absorbs a new spec hole).
  **The ONE mechanical anti-mush assert that DOES ship (structurally hole-free) is a
  RELIEF-CONSERVATION FLOOR on the 64² fixture:**
  `p90(h_post) − p10(h_post) ≥ 80% · (p90(h_pre) − p10(h_pre))`, restricted to each mask's
  cells — no crest set, no strict-maxima degeneracy, no div-by-zero (the pre-spread is large
  by construction), i64-trivial (`100·spread_post ≥ 80·spread_pre`). The in-suite 64² test
  asserts: step bound both-directions (as above) ∧ the relief-conservation floor per mask. **`LandformMasks { edifice, till, dune: Vec<bool> }` is
  built INSIDE caps.rs from what it ALREADY computes** — `volcanic_mask.is_some()`
  (caps.rs:402), `glacial_mask == Some(Till)` (caps.rs:409), `sand_depth[i] > 0` (caps.rs:416;
  `sand_depth>0` IS the persisted dune set per caps.rs:313 — NOT `wind_shadow_mask`, which is
  a per-CA-iteration leeward pickup-suppression mask over a field aeolian saw mid-pipeline,
  neither persistent nor a deposit set) — and returned by the staged output
  (`classify_and_caps_staged` carries it; OFF path = all-empty masks, default golden path
  untouched). Zero re-derivation, zero new height reads: the sweep bin and the in-suite 64²
  test consume the SAME returned masks, computed ONCE on the PRE-talus state and passed to
  both the pre- and post-pass amplitude calls (talus moves `height` only, so the region sets
  are identical; a re-derived mask — e.g. `ice_mask` reads height, glacial.rs:123 — would move
  between calls and compare different cell sets). Production placement (`world::gen`) is
  justified by reviewability/stability AND by its in-suite consumer: the relief-conservation
  floor test reads the SAME masks and helpers — one definition, no drift; a bin-local metric
  would force the suite to re-invent it and the cheapest proxy (mean height) is vacuous under
  a Σ-conserving pass.
- Sweep (cloud or DIM=64 local + 512 cloud, same pattern as glacial 0b): threshold ∈ {8,12,16,24}
  × iters ∈ {2,4,8}. **The GATE is ABSOLUTE, decoupled from the swept knob** (a thr-relative
  gate is the diffusion's own fixed point — it loosens as thr grows, and "smallest smoothing"
  would converge on thr=24 whose passing field carries residual ~30-unit single-column steps,
  i.e. the picket fence itself, gate-green): **`pub const MAX_LOCAL_STEP_FINAL: i64 = 12`**
  (≤ NEEDLE_MARGIN/2). Gate = needles==0 ∧ no strict local max exceeds its highest **D8**
  neighbor by > MAX_LOCAL_STEP_FINAL (D8 explicitly — de_needle's nmax is D8, erosion.rs:257;
  a D4-measured gate would NOT imply the no-op). Landform-amplitude retention (p10, ≥60%
  guideline) is REPORTED per config and reviewed by the human at pick — it is a decide-metric,
  not a mechanical gate (see the metric spec above). **PICK SET = {thr : thr ≤ 8} — pick
  collapses to thr=8 × fewest iters passing the gate, with the human confirming the reported
  retention numbers.** 12/16/24 stay in the grid REPORT-ONLY (controls, never pickable): the fixed point
  leaves drops ≈ thr + the ≤1-unit un-scale floor, so thr=12 lands AT the gate boundary
  (residual 13 vs bound 12 — an integer coin-flip that would freeze a zero-margin config into
  a shipping assert). The gate's nmax is measured with the SAME in-grid D8 neighbor skip as
  de_needle (erosion.rs:289) — a padded/clamped border measurement would not imply the no-op
  at borders. **The coupling is MECHANICAL, not editorial:** hoist
  `NEEDLE_MARGIN` to `pub const`; define `MAX_LOCAL_STEP_FINAL` and the picked
  `REPOSE_THRESHOLD_FINAL` next to it in erosion.rs; add
  `const _: () = assert!(MAX_LOCAL_STEP_FINAL < NEEDLE_MARGIN);` (zero runtime cost; a
  gate-passing field has every local-max step ≤ 12 < 30 ⇒ de_needle no-op GUARANTEED, and the
  bound is not a sweepable knob; NEEDLE_MARGIN itself is pinned by two existing tests —
  erosion.rs:772 asserts a 110-spike clips to ≤ nmax+30 ⇒ margin ≤ 30, erosion.rs:792 the
  converse ⇒ margin ≥ 30 — cite them); plus one in-suite test asserting the
  MAX_LOCAL_STEP_FINAL bound on a 64² landform-ON fixture. **The 64² fixture is REGENERATED
  from pinned params (seed, dim=64, all landforms ON — the sweep's own dim-64 cell params;
  NOT a checked-in snapshot, which would stop tracking worldgen), same convention as the
  existing golden vectors. The test asserts MECHANICAL properties only (step bound; NO
  amplitude assert — amplitude is report-and-decide per the metric spec) — there is no pinned
  value, and relaxing an assert is never the response to a landform change that breaks it
  (diagnose instead). The test pins BOTH directions: the PRE-talus field VIOLATES the
  ABSOLUTE bound (non-vacuity — else a no-op talus passes silently; the bound is a const, not
  the swept thr, so the assert is not self-referential) AND the POST-talus field satisfies
  it.** One
  artifact, one gate, no dim mismatch; the sweep reports the gate at dim=64 too, not only 512.
  **Empty-grid action (pre-decided, BRANCHED ON THE FAILING SIGNAL):** reported retention
  below guideline — ALONE OR JOINTLY with anything else → STOP, human decision point (more
  iters only lowers retention further; a joint step+retention red is NOT a license to take
  the extend branch); ONLY a pure step-gate and/or needles-gate failure (retention numbers
  healthy) → EXTEND iters (diffusion is local, cost linear); NEVER raise
  MAX_LOCAL_STEP_FINAL. Surfaced with the numbers — not a knob the coder turns. NO
  global-p99-step gate: linear Jacobi diffusion has a constant-gradient ramp as its fixed
  point — an extended steep face (volcanic flank, coastal scarp) erodes only from its ends,
  O(width²) iterations, so a global step bound is likely unsatisfiable jointly with healthy
  amplitude retention at iters ≤ 8. Broad steep faces are LEGITIMATE RELIEF; the pass targets
  1-cell needles and local spikes, which linear diffusion kills fast (excess halves per
  iteration).
- Gated `any_landform_on` (same gate as de_needle) → conserved goldens untouched by construction.
- **Sweep metrics are measured on the TALUS-ONLY field (de_needle excluded).** Production runs
  de_needle LAST (caps.rs:458, NEEDLE_MARGIN=30 clips every cell > nmax+30), so a
  post-de_needle field shows few/no needles REGARDLESS of what talus did
  — a sweep on it is degenerate and "smallest smoothing" would reward a no-op talus (picket
  fence survives under the clip).
- **The staged seam is IN W-9's SCOPE (not smuggled into a throwaway bin):** today NO seam
  exists — caps.rs:458 applies de_needle unconditionally under the landform gate and
  `classify_and_caps` returns only the post-de_needle `.height` (`post_coastal_height` is a
  function local). W-9 adds a staged output (`classify_and_caps_staged` →
  `{post_coastal, post_talus, post_deneedle, masks: LandformMasks}` — the masks ride the SAME
  seam (F39: a 3-height struct would leave the amplitude metric with no mask source); pure
  addition; `classify_and_caps` becomes a thin wrapper; OFF path byte-identical, masks
  all-empty on OFF; cli/map_dump call sites listed and untouched semantically). The sweep bin reads `post_talus`; the GATE reads the post-talus field. This
  caps.rs signature work is named in W-9 acceptance.
- de_needle_pass: KEPT UNCONDITIONALLY in production. It is one O(N) pass and a no-op at any
  gate-passing config (guaranteed by the admissibility constraint above) — retiring it buys
  nothing and would drop the only guard on dims/seeds the sweep never tested. The sweep still
  REPORTS its clip count on the post-talus field (==0 at the picked config is an ACCEPTANCE
  item; >0 on an untested seed later is the guard doing its job, not a failure).
- Determinism: fixed N iterations, Jacobi double-buffer (read `h_old`, write `h_new`), integer
  only, fixed row-major pair order. `talus_step_final` is a NEW function per the exact rule
  above; the existing `talus_step` is not called, not modified, not parameterized.
- Acceptance: needles==0 @512×2 seeds ON THE POST-TALUS FIELD; local-max step ≤
  MAX_LOCAL_STEP_FINAL (post-talus); per-landform p10 retention REPORTED for every swept
  config and the picked config's numbers ATTACHED to the PR (≥60% guideline; human-reviewed
  at pick — no shipping assert); **`classify_and_caps_staged` seam LANDS CARRYING
  `masks: LandformMasks` (pure
  addition, `classify_and_caps` = thin wrapper, OFF path byte-identical with all-empty masks,
  cli/map_dump call sites untouched semantically)**; **de_needle clip count == 0 at the picked
  config @512×2 seeds**; per-iteration range-contraction test + needle-fixture test in-suite;
  **the `const _` static assert (MAX_LOCAL_STEP_FINAL < pub NEEDLE_MARGIN) + the 64²
  both-directions local-max-step in-suite test BOTH land**; conserved goldens byte-identical;
  landform-ON golden vector re-pin two-pass; compile-check PASS.

**W-10 — material diversity (the "почвы различного состава" ask) — PRESENTATION-ONLY split.**
`MaterialId` inside classify remains the SIM SUBSTRATE — the biome cascade (`override_biome`,
incl. the Fertile branch caps.rs:137) and `caps_from`/`material_mult` read the UNCHANGED
substrate; touching it would silently rewrite caps/O₂/NO₃ ecology (Fertile branch dead for renamed cells; Bedrock (0,1)
cap collapse on steep-wet cells). The split lives ONLY in the presentation byte that
`surface_material` returns:
- (a) Soil → {SoilDry, Soil, SoilWet} in the PRESENTATION byte, keyed on the ALREADY-COMPUTED
  moisture at thresholds picked from the moisture histogram @512 (measure first, same bin as
  W-9 Phase-0). Two new u8 discriminants APPENDED (9, 10); the substrate cell stays `Soil` for
  every downstream consumer (biome, caps, resources).
- (b) Exposed-rock/outcrop by slope: where slope (already computed, caps.rs:495) ≥ threshold
  and the presentation byte would be Soil* → presentation byte = Bedrock (scree/outcrop look).
  Substrate untouched; landform-primary bytes (Basalt/Tuff/Till/Sand) keep priority.
- Mechanism: presentation byte derived AFTER classify from (substrate, moisture, slope) — a
  pure post-pass writing the render-facing field only. TESTABLE claim: ON-path biome/caps/
  resource arrays byte-identical pre/post W-10 (this test ships in the slice).
- (c) WorldView is NOT extended; render reads only `surface_material`. No sim-core change.
- Behind the SAME landform-ON gate (OFF path byte-identical ⇒ w2/w5 + conserved goldens stand).
- Acceptance: @512 all-ON, 2 seeds: ≥7 distinct presentation materials PRESENT with coherent
  patches (patch count / mean patch size reported — no salt-and-pepper), screenshot evidence of
  visual distinctness; NO share-floor KPI and NO tuning of vents/ELA/landform physics to
  manufacture material coverage (anti-forcing: physical knobs are not render KPIs);
  ON-path biome/caps byte-identity test; OFF-path byte-identity; re-pin two-pass (golden
  vectors that hash the surface_material byte only); compile-check PASS.

Order: W-9 → W-10 (W-10's slope/moisture stats move after relaxation; measure on post-W-9 field).

### Lane R (coder B — render, out of CI)

**R-13 — evidence + bench harness (FIRST: unblocks all visual verification).**
- `--screenshot <path.png>` : render N warmup frames headed, dump framebuffer to PNG, exit 0.
  (macroquad `get_screen_data()` → `export_png`; must run headed on macOS — document.)
- `--bench` : fixed camera path (3 zoom levels × 2 yaws), report avg/p95 frame ms over ≥300
  frames as a machine-readable line (`BENCH dim=512 avg_ms=… p95_ms=…`); exit 0.
- Deterministic camera preset flag (`--cam iso-default`) so screenshots are comparable across PRs.
- Acceptance: screenshot PNG reproducible (same seed/dim/cam ⇒ visually identical); bench line
  parseable; clippy green; compile-check (render workspace) PASS.

**R-14 — look pack (the diorama).** All px-verified via R-13 screenshots against the reference.
- Per-vertex AO: for each top-face vertex, darken by count/height of strictly-higher hex
  neighbors touching that corner (bake at mesh build into vertex color; no shader change).
  Cliff quads: depth-darken toward their base (v1-style).
- Top bevel: chamfer ring — shrink top hexagon by BEVEL_FRAC (~0.12·HEX_SIZE), add 6 chamfer
  quads to the outer rim; +12 tris/cell worst case. Slight normal tilt on chamfer for the light
  to catch (this is what makes the reference read "toy").
- Chunk-capacity contract (IN THIS SLICE, not R-15): a `const VERTS_PER_CELL_MAX` PER MESH
  KIND (hex-with-chamfer ~54; cube stays ~30 — `terrain_cube.rs:27` shares
  `terrain::rows_per_chunk`, and a global 30→54 bump would needlessly halve cube chunks),
  used by BOTH each mesh builder and its `rows_per_chunk` (terrain.rs:24 currently hardcodes
  30 — at dim=512 rpc=3 ⇒ ~83k verts/chunk > the 60k drawcall buffer (main.rs:142) ⇒ macroquad
  silently DROPS trailing geometry). Update the divisor per kind,
  `assert!(chunk_verts < 60_000 && chunk_indices < 120_000)` at build (the two
  `gl_set_drawcall_buffer_capacity` slots at main.rs:142 — vertex capacity 60k, index SLOTS
  120k; `u16::MAX` bounds index *values* and is already implied by verts<60k) — a HARD
  `assert!`, not `debug_assert!`: every evidence build (screenshot/BENCH) is `--release`,
  where `debug_assert!` compiles out; mesh build is once at startup, so the check is free.
  The hard `assert!` carries a message COMPUTED from the mesh kind's own VERTS_PER_CELL_MAX
  (there is no single max dim — hex ≈1111, cube ≈2000; a literal would lie for one builder),
  else dim=1200 panics cryptically at startup. Two constants, two ROLES, stated in-code: the per-kind `rows_per_chunk` formula keeps the
  SOFT budget 50_000 (headroom under the cap); the `assert!` uses the HARD caps
  (60_000 verts / 120_000 index slots, main.rs:142). VERTS bind before indices (dim×54 verts
  vs dim×84 indices against 60k/120k — that is WHY a verts-only rows_per_chunk formula is
  sound). In-code doc states BOTH ceilings honestly: soft budget exhausted at dim ≈ 925 (rpc
  clamps to 1, chunk exceeds 50k but ships), hard abort at dim ≈ 1111 (the assert fires). No
  column-split fallback exists.
- Palette v2: color = material HUE × height-tier VALUE ramp (two-factor; replaces either/or
  ColorMode) + deterministic per-column value jitter `hash(col,row,seed) ∈ ±4%`. Materials to
  cover: Bedrock (cool grey), Sand (warm tan), SoilDry (pale ochre), Soil (mid brown),
  SoilWet (dark umber), Till (grey-blue), Basalt (near-black), Tuff (light brown),
  Permafrost (ice grey). Keep 'C' toggle: material-hue mode ↔ pure hypsometric (debug).
- `--bare` mode (default ON for this program's screenshots): Water material → dry-bed tint
  (desaturated sand), no other change.
- Backdrop: vertical gradient sky + subtle distance fog (cheap, sells the diorama); OFF-toggle.
- HEIGHT_SCALE: re-tune (0.2 was set to tame the picket fence; after W-9 smoothing try 0.3–0.4
  for drama). Screenshot A/B at 0.2/0.3/0.4 attached to PR — user picks final.
- Acceptance: screenshot set @512 all-ON on the POST-W-9 map (slice is blocked on W-9 merge —
  see Dispatch), clippy, compile-check PASS. Beauty verdict = user (PM attaches screenshots
  to PR).

**R-15 — perf at 512² (gate: 60fps) — the bottleneck is BANDWIDTH, not triangles.**
macroquad's `draw_mesh` re-copies verts and REWRITES indices CPU→GPU EVERY FRAME (main.rs:134-136
own comment; no retained VBO). Full-map iso view ⇒ ~262k cells × ~54 verts ≈ 14M verts streamed
per frame — that cannot hit 60fps regardless of triangle budget.
- Lever 1 (primary): RETAINED GPU BUFFERS via the miniquad raw API — upload each chunk's mesh
  once (immutable buffers), draw per frame. The exact pattern already exists IN-REPO: v1
  `crates/animata/src/render/gpu.rs` (retained immutable buffers, 1 draw call per visible
  chunk). Port the pattern, not the code. This is NOT a wgpu rewrite.
- Lever 2 (if still short at full-map zoom): coarse far tier — stride-sampled whole-map mesh
  swapped in beyond a zoom threshold (v1 streamer's coarse-tier idea, simplified: static, no
  streaming). Bevel/AO stay on the near tier only.
- Baseline first: R-13 bench @512 all-ON BEFORE levers (numbers drive how far to go).
- Target: avg ≤ 16.6ms, p95 ≤ 20ms at all three bench zoom levels.
- Acceptance: BENCH lines in PR body @512, before/after per applied lever; stop at the first
  lever that hits target. PLUS screenshot PARITY: `--screenshot` at `--cam iso-default` (all
  three zoom levels) pre- vs post-retained-path must be pixel-identical (or diff-explained) —
  Lever 1 is a draw-path rewrite (own GLSL/MVP/Pipeline state, leaves macroquad `draw_mesh`);
  a depth/cull/format slip would silently change the user-approved R-14 look, and this lane
  has no CI to catch it. R-13 makes this check free.

### Housekeeping (PM, no code authored)
- H-1: commit the uncommitted bridge working tree AS-IS on the integration branch (compile-check
  ×3 workspaces first — running now). Message marks it exploratory tooling.

## Dispatch / sequencing
- A: W-9 → W-10. B: R-13 → (wait for W-9 merge) → R-14 → R-15. Lanes parallel on disjoint
  crates (D5); the ONE ordering edge: **R-14 is BLOCKED on W-9 merged** — its look tuning
  (HEIGHT_SCALE pick, AO/bevel judged) must happen on the post-relaxation map, otherwise every
  aesthetic decision is made against the picket-fence surface and silently goes stale (nothing
  in the render lane's out-of-CI gate would force a re-shoot). If W-9 stalls, B idles or takes
  R-15 Lever-1 groundwork (retained buffers are look-independent).
- Issue per slice (repo convention), ТЗ via tz-author grounding, coder self-review via
  code-critic pre-ready, PM code-critic at intake, done-gate contract as per CLAUDE.md.

## Program acceptance (user-facing)
512² all-ON map: ≥60fps (BENCH evidence); screenshot set shows smooth-yet-diverse relief
(no needles/picket fence), ≥7 visually distinct bare materials, no water/vegetation, diorama
look (AO + bevel + per-column variation) comparable in spirit to the reference image.
**FINAL EVIDENCE PASS (owner: coder B, a named checklist item on the LAST slice to merge):**
after W-9 ⊕ W-10 ⊕ R-14 (and R-15 if run) are ALL merged, regenerate the full screenshot set
@512×2 seeds on the merged integration branch and attach to PR #428 — R-14's own screenshots
predate W-10's SoilDry/SoilWet bytes (R-14 is blocked only on W-9), so without this pass the
user signs off a look that never showed the final materials, and nothing out-of-CI would force
a re-shoot.

## Out of scope
- v1 render/gen changes of any kind; productizing the bridge beyond H-1.
- GPU instancing / wgpu rewrite for v2 render. (Retained miniquad buffers + a static coarse
  far tier are IN scope via R-15; dynamic LOD *streaming* à la v1 streamer.rs is not.)
- Vegetation, water rendering, rivers, caves.
- WorldView trait extension (W-10c explicitly deferred).
- Master merge (stays on integration branch until user says go).

## Accepted operational risks (trade-offs)
- **[F18, critic severity: tradeoff] talus un-scale floor loss.** `h = hs/64` floors ≤1
  unit/cell; the FINAL field is not Σ-conserved (up to ~262k units @512). Accepted: gated
  landform-ON (conserved goldens untouched), Σ-invariance holds inside the scaled loop where
  it is testable, and the post-unscale asserts check the properties that matter (needles,
  step bound). The plan explicitly does NOT bill the final field as conserved.
- **[PM decision after 16 critic rounds] amplitude retention is report-and-decide, not a
  shipping assert.** Ten consecutive rounds each found a new spec hole in the frozen
  amplitude assert (self-reference, set-max escapes, div-by-zero, float-guard, sheet vs
  crest…). The protection against over-smoothing is now: sweep REPORTS per-landform p10
  retention (one crest-list primitive, i64), human reviews the numbers at config pick, STOP
  branch on low retention. Residual risk: a future re-run without human review could pick a
  mushing config — mitigated by the pick procedure requiring the numbers in the PR.
- **[F8-adjacent, render] 60fps at full-map zoom depends on R-15 Lever 1 (retained buffers).**
  If the port stalls, the fallback is the static coarse far tier (Lever 2); interactive zoom
  levels are safe under frustum culling either way.
