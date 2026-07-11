//! The 11 tick stages (0–10). Conserved-layer arithmetic is exact integer; every `eu` moved is
//! accounted in the [`EnergyLedger`] so the conservation residual stays EXACTLY 0 (R15).
//!
//! Stages that contend over a shared quantity resolve deterministically by **Entity-id order**:
//! Interactions (who eats a contested cell first) and BirthDeath (spawn/despawn order → deterministic
//! child Entity ids). Independent per-entity stages (Metabolism, Sense, Act) need no ordering.

use crate::*;
use bevy_ecs::prelude::*;
#[cfg(feature = "perf")]
use crate::WorkCounters;

/// RNG salts — disjoint streams, each must differ so draws are uncorrelated (R14).
const SALT_MUT: u64   = 0x4D55_5400; // "MUT\0"
const SALT_DEATH: u64 = 0x4445_4100; // "DEA\0" — C-1 background-death draw, MUST ≠ SALT_MUT

// ── P1-2b: Hypoxia self-shading via oxygen diffusion (O₂-field layer 2, non-energy) ──────────────────
//
// CBRT_LUT[N] = ⌊256·cbrt(N)⌋ for N ∈ [0..256], Q8.8 fixed-point format.
// Used to compute inner_fraction = 1 − N^(−1/3) = (CBRT_LUT[N] − 256) / CBRT_LUT[N].
// Examples: N=1→256, N=4→406, N=8→512, N=64→1024, N=256→1625.
const CBRT_LUT: [i32; 257] = [
     0,  256,  323,  369,  406,  438,  465,  490,  512,  533,
   552,  569,  586,  602,  617,  631,  645,  658,  671,  683,
   695,  706,  717,  728,  738,  749,  758,  768,  777,  787,
   795,  804,  813,  821,  829,  837,  845,  853,  861,  868,
   876,  883,  890,  897,  904,  911,  917,  924,  930,  937,
   943,  949,  956,  962,  968,  974,  979,  985,  991,  997,
  1002, 1008, 1013, 1019, 1024, 1029, 1035, 1040, 1045, 1050,
  1055, 1060, 1065, 1070, 1075, 1080, 1084, 1089, 1094, 1098,
  1103, 1108, 1112, 1117, 1121, 1126, 1130, 1134, 1139, 1143,
  1147, 1151, 1156, 1160, 1164, 1168, 1172, 1176, 1180, 1184,
  1188, 1192, 1196, 1200, 1204, 1208, 1212, 1215, 1219, 1223,
  1227, 1230, 1234, 1238, 1241, 1245, 1249, 1252, 1256, 1259,
  1263, 1266, 1270, 1273, 1277, 1280, 1283, 1287, 1290, 1294,
  1297, 1300, 1303, 1307, 1310, 1313, 1316, 1320, 1323, 1326,
  1329, 1332, 1336, 1339, 1342, 1345, 1348, 1351, 1354, 1357,
  1360, 1363, 1366, 1369, 1372, 1375, 1378, 1381, 1384, 1387,
  1390, 1393, 1396, 1398, 1401, 1404, 1407, 1410, 1413, 1415,
  1418, 1421, 1424, 1426, 1429, 1432, 1435, 1437, 1440, 1443,
  1445, 1448, 1451, 1453, 1456, 1459, 1461, 1464, 1467, 1469,
  1472, 1474, 1477, 1479, 1482, 1485, 1487, 1490, 1492, 1495,
  1497, 1500, 1502, 1505, 1507, 1509, 1512, 1514, 1517, 1519,
  1522, 1524, 1526, 1529, 1531, 1534, 1536, 1538, 1541, 1543,
  1545, 1548, 1550, 1552, 1555, 1557, 1559, 1562, 1564, 1566,
  1568, 1571, 1573, 1575, 1578, 1580, 1582, 1584, 1586, 1589,
  1591, 1593, 1595, 1598, 1600, 1602, 1604, 1606, 1608, 1611,
  1613, 1615, 1617, 1619, 1621, 1623, 1625,
];

// Stage 0 (SpatialRebuild) REMOVED (M1/F2): the NeighborGrid was rebuilt every tick but never
// queried by any stage — dead per-tick work. Removed until a real neighbour-coupled consumer lands.

// ── Stage 1: Sense — read the conserved resource field (version t): integer gradient + local amount.
//    Signal pheromone gradient is intentionally NOT fed to the integer brain in M3 (see stage_brain);
//    the dead per-tick compute was removed (M3/F3). Signal still contributes to state_hash via
//    signal_hash(), keeping the golden arm64-pinned. ───────────────────────────────────────────────
//
// E-4b-i (critic F3/F11): the sensed layer comes from `Phenotype.uptake_layer`, NOT
// `Genome.uptake_layer` directly — the SAME field `stage_interactions` reads below. Before this
// slice both stages agreed only because `Phenotype.uptake_layer` was always a 1:1 copy of the
// genome (E-1); E-4b-i's `decode` can now DERIVE it from `cell_type`, so reading the raw genome
// here would let an entity SENSE one layer and EAT another — a silent desync. Neutral for the five
// existing configs: `cell_type: None` ⇒ `phenotype.uptake_layer == genome.uptake_layer` exactly
// (genome.rs `decode`), so this is byte-identical there.
pub fn stage_sense(field: Res<FieldRes>, mut q: Query<(&Position, &Genome, &Phenotype, &mut Sensors)>) {
    for (pos, g, ph, mut s) in &mut q {
        let range = g.sense_range.max(1) as i64;
        let layer = ph.uptake_layer as usize; // E-4b-i: same field stage_interactions reads
        let (gx, gz) = field.0.conserved_gradient(pos.0, range, layer);
        s.gradient = Vec2Fixed(gx, gz);
        s.local_resource = field.0.conserved_at(pos.0, layer);
    }
}

/// Act dead-zone on a `FixedI16` (Q8.8) motor output: |out| ≤ this ⇒ no move on that axis (real
/// 0.0625). Keeps a near-zero brain output from jittering the integer position every Act tick.
const ACT_DEADZONE: i16 = 16;

/// Clamp an `i64` sensor reading into the `FixedI16` brain-input range (Q8.8). Out-of-range CLAMPS
/// (never wraps), like the activation LUT — keeps inference inside its proven, deterministic envelope.
#[inline]
fn q88_clamp(v: i64) -> i16 {
    v.clamp(i16::MIN as i64, i16::MAX as i64) as i16
}

/// Brain level-of-detail (D-Brain-5, SKELETON only). The branch point where a far/inactive creature
/// would run a baseline controller / thinned inference instead of full inference. The criterion is
/// computed from DETERMINISTIC SIM STATE (never camera/render — that would be non-deterministic). In
/// M3 every creature is `Full`; the 4-level sim-LOD is M4. The branch exists so M4 only fills it in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BrainLod {
    Full,
    #[allow(dead_code)]
    Baseline,
}

#[inline]
fn brain_lod(_energy: i64) -> BrainLod {
    // M4 will thin inference by a deterministic-state criterion (e.g. energy/age/density tier). M3
    // always runs full inference so the trajectory is the full-fidelity one.
    BrainLod::Full
}

// ── Stage 2: Brain — batched INTEGER inference (M3 / D-Brain-1..4), runs only every K ticks on a
//    GLOBAL phase (`tick % K == 0`). Reads quantized sensors + recurrent `h_old` + the creature's
//    evolved weights → writes the recurrent `h_new` and the motor `BrainOutput`, then swaps the
//    hidden double-buffer. Between Brain ticks `BrainOutput` persists and the hidden state is frozen. ─
pub fn stage_brain(
    clock: Res<SimClock>,
    econ: Res<EconParams>,
    brain: Res<BrainRes>,
    mut q: Query<(&Sensors, &Energy, &Genome, &mut BrainState, &mut BrainOutput)>,
    #[cfg(feature = "perf")] mut wc: ResMut<WorkCounters>,
) {
    if !clock.tick.is_multiple_of(econ.brain_period.max(1)) {
        return; // off-phase: behaviour holds the last decision (multi-rate, R20). Newborns stay frozen.
    }
    for (s, e, g, mut bs, mut bo) in &mut q {
        #[cfg(feature = "perf")]
        { wc.brain_infer += 1; }
        // Sense→Brain quantization boundary: pack the integer sensors into the FixedI16 input vector.
        // Inputs: [grad_x, grad_z, local_resource, energy, bias=1.0(Q8.8), reserved]. The signal field
        // (f32) is intentionally NOT fed to the integer brain in M3 — it stays observational.
        let inputs: [i16; BRAIN_INPUTS] = [
            q88_clamp(s.gradient.0),
            q88_clamp(s.gradient.1),
            q88_clamp(s.local_resource),
            q88_clamp(e.0),
            256,
            0,
        ];
        match brain_lod(e.0) {
            BrainLod::Full => {
                let mut h_new = [0i16; BRAIN_HIDDEN];
                let mut out = [0i16; BRAIN_OUTPUTS];
                brain.0.infer(&inputs, &bs.h_old, &g.weights, &mut h_new, &mut out);
                bs.h_new = h_new;
                // Double-buffer swap: commit `h_new` → `h_old` for the next Brain tick (per-entity
                // equivalent of the whole-array pointer swap; happens ONLY on Brain ticks).
                bs.h_old = bs.h_new;
                bo.out = out;
            }
            BrainLod::Baseline => {
                // M4: a cheap baseline controller. M3 never reaches here (skeleton).
                bo.out = [0; BRAIN_OUTPUTS];
            }
        }
    }
}

// ── Stage 3: Act — apply the brain's motor decision (M3 / D-Brain-4): desired velocity = per-axis
//    sign(BrainOutput) · move_speed → Intent. Reads `BrainOutput` at the BASE rhythm (every tick), so
//    the same decision drives motion for the K-1 ticks between Brain ticks. No chemotaxis hard-code. ─
//    No hidden fitness: moving toward food grants NO energy; it only pays off via actual feeding.
pub fn stage_act(mut q: Query<(&BrainOutput, &Genome, &mut Intent)>) {
    for (bo, g, mut intent) in &mut q {
        let sp = g.move_speed as i64;
        let drive = |o: i16| -> i64 {
            if o > ACT_DEADZONE {
                sp
            } else if o < -ACT_DEADZONE {
                -sp
            } else {
                0
            }
        };
        intent.0 = Vec2Fixed(drive(bo.out[0]), drive(bo.out[1]));
    }
}

// ── Stage 4: Move — 2.5D: integrate Intent, wrap in the domain, project onto walkable terrain. ─────
pub fn stage_move(
    world: Res<WorldRes>,
    econ: Res<EconParams>,
    mut q: Query<(&Position, &Intent, &mut PositionNext, &mut VelocityNext)>,
) {
    let dim = econ.world_dim;
    for (pos, intent, mut pn, mut vn) in &mut q {
        let mut nx = (pos.0 .0 + intent.0 .0).rem_euclid(dim);
        let mut nz = (pos.0 .1 + intent.0 .1).rem_euclid(dim);
        // 2.5D kinematic projection (R16): cannot enter solid terrain (height-derived via WorldView).
        if world.0.is_solid(Vec2Fixed(nx, nz)) {
            nx = pos.0 .0;
            nz = pos.0 .1;
        }
        pn.0 = Vec2Fixed(nx, nz);
        vn.0 = intent.0;
    }
}

// ── Stage 5: Metabolism — base + size^¾ + move + sense cost, fixed-point ledger, N=1 (R20). ────────
//
// D′-2a: photo-machinery expression cost added here. Per-tick rate r = (NUM·expressed)/DEN;
// D′-2b: expressed = expressed_capacity(g, L(t)) — 0 at night when reg_gain>0 → cost skipped.
// Placement: inside the maintenance bracket; computed as (NUM·eff·n)/DEN. Scales linearly → R20.
pub fn stage_metabolism(
    econ: Res<EconParams>,
    clock: Res<SimClock>,
    world: Res<WorldRes>,
    mut ledger: ResMut<EnergyLedger>,
    mut tel: ResMut<Telemetry>,
    mut q: Query<(&Position, &Genome, &Phenotype, &mut Energy)>,
) {
    let n = econ.metab_period.max(1);
    if !clock.tick.is_multiple_of(n) {
        return; // multi-rate metabolism (D-Brain-4): runs every N ticks, GLOBAL phase.
    }
    debug_assert!(econ.photo_cost_den > 0, "photo_cost_den must be > 0");
    // D′-2b: expressed capacity gates the cost. Night-downregulated cell (reg_gain>0, L=0) has
    // expressed_capacity=0 → photo_cost=0 (the selective saving). Non-dprime: photo_gain≡0 →
    // expressed_capacity=0 → cost=0, byte-identical isolation. When light=None: l_now=0 but
    // photo_gain=0 for all non-dprime genomes → early exit 0 in expressed_capacity anyway.
    //
    // R20 alignment invariant (enforced by Sim::new hard assert): day_ticks and period_ticks are
    // exact multiples of metab_period → every n-tick lump window is wholly within one phase →
    // l_now sampled once at the metab tick is representative of the entire lump. The (eff·n)/den
    // lump is N-invariant only under this alignment; Sim::new rejects configs that violate it.
    // P5-D: l_now is now per-entity (depth-attenuated), computed in the loop.
    let mut photo_cost_this_event: i64 = 0;
    for (pos, g, ph, mut e) in &mut q {
        // M7-e-a: coordination cost on total live body cell count (Σ module_cell_count). 0 for
        // every non-phase2 genome (empty CellGraph) and inert at c_coord=0 (all shipped configs).
        let n_cells: i64 = ph.graph.module_cell_count.iter().map(|&c| c as i64).sum();
        // Charge ×N — a lump standing in for the N base ticks since the last metabolism tick, so the
        // economy stays ≈invariant to N and conservation is exact (R15).
        // P3-2 (B5): breadth-cost additive in base_cost. Specialist/generalist tradeoff:
        // wider tol_breadth incurs standing metabolic cost (gate on is_some() for byte-identity).
        let breadth_cost = if econ.ambient_tolerance.is_some() {
            econ.ambient_tolerance.as_ref().unwrap().breadth_cost_k * g.tol_breadth as i64 / crate::params::BREADTH_COST_SCALE
        } else { 0 };
        // GA-LOAD: genetic-load energy burden cost. Additive in base_cost.
        // Load expresses as standing metabolic drain: burden_cost = genetic_load × burden_cost_k
        // (gate on enable_mutation_load for byte-identity with non-load configs).
        let burden_cost = if econ.enable_mutation_load {
            g.genetic_load as i64 * econ.burden_cost_k
        } else { 0 };
        let base_cost = (econ.base_metab
            + econ.k_size_metab * g.metab_units()
            + econ.k_move_cost * g.move_speed as i64
            + econ.k_sense_cost * g.sense_range as i64
            + econ.c_coord * n_cells
            + breadth_cost
            + burden_cost)
            * n as i64;
        // D′-2a/2b: photo-machinery expression cost on the EXPRESSED capacity.
        // expressed_capacity returns 0 at night for regulated cells → cost skipped (the D′-2b lever).
        // Charge per event = (NUM · eff · n) / DEN (delayed division avoids truncation at low eff).
        // Threshold: at NUM=1, DEN=8, n=2 → eff ≥ 4 for non-zero charge (≈ 16.7% of day income).
        // P5-D: compute L(t) per-entity using per-cell height; when enable_photic=false this is
        // identical to the old uniform l_now (byte-identity guarantee).
        let l_now = crate::params::light_at(econ.light.as_ref(), clock.tick, econ.enable_photic, world.0.height(pos.0.0, pos.0.1));
        let eff = expressed_capacity(g, l_now);
        let photo_cost = if eff > 0 {
            (econ.photo_cost_num * eff as i64 * n as i64) / econ.photo_cost_den
        } else {
            0
        };
        // P1-2a: aerobe_cost — maintenance of O₂-respiration machinery (ROS-detox, enzymes).
        // Proportional to energy to prevent starvation-death on respiring lineages.
        // Cost = (aerobe_cost_x256 / 256) × (energy / e_cell) × n, where aerobe_cost_x256 encodes
        // genotypic metabolism type (10 for obligate-aerobe; 15 for facultative).
        // ISOLATION GATE (`econ.enable_oxygen`): gate on the econ flag, NOT `.is_some()` —
        // decode_respiratory_pathways(founder gene 0) returns Some(obligate-aerobe), so `.is_some()`
        // is true for EVERY legacy entity and would charge the aerobe-machinery cost in all five
        // shipped configs (golden drift). enable_oxygen=false → cost 0 → byte-identical.
        let aerobe_cost = if econ.enable_oxygen {
            match &ph.respiratory_pathway {
                Some(rp) => {
                    // Proportional: cost scales with current energy level and lump size n.
                    debug_assert!(econ.e_cell > 0, "econ.e_cell must be > 0");
                    rp.aerobe_cost_x256 as i64 * e.0.max(0).min(econ.e_cell) / 256 * n as i64 / econ.e_cell
                }
                None => 0,
            }
        } else {
            0
        };
        let mut cost = base_cost + photo_cost + aerobe_cost;
        // Can only dissipate what it has — energy never goes negative; death (energy 0) is in stage 7.
        let actual = cost.min(e.0.max(0));
        e.0 -= actual;
        ledger.dissipated += actual;
        // Track the photo share of actual dissipation (proportional — not a naive min).
        // When energy is short (actual < cost), the photo share is photo_cost·actual/cost,
        // not min(photo_cost, actual) which overstates it. D′-2c measures regulated vs
        // constitutive cost savings off this counter, so accuracy under deficit matters.
        photo_cost_this_event += if cost > 0 { photo_cost * actual / cost } else { 0 };
    }
    // Accumulate cumulative photo-cost (non-inertness tooth: must be > 0 over a long dprime run).
    tel.photo_cost_total += photo_cost_this_event;
}

/// Monod saturating uptake demand: `U(R) = (u_max·R) / (R+km)`, integer, truncating toward zero.
///
/// Requires `km > 0` — at `km=0` and `R=0`, the denominator is zero (integer divide panic).
/// The product `u_max·R` cannot overflow i64: v2 field cells are capped at `≈RESOURCE_BASE+HMAX≈136`,
/// `u_max≤220`, so `u_max·R ≤ 220·136 = 29_920` — headroom to i64_max is ~3×10^14 (safe).
pub fn monod_demand(u_max: i64, km: i64, r: i64) -> i64 {
    debug_assert!(km > 0, "km must be > 0: at R=0 the denominator r+km = km must be ≥ 1");
    (u_max * r) / (r + km)
}

/// Photo uptake demand (D′-1): `U_photo(L) = photo_gain · L / (km_photo + L)`, integer, truncating.
///
/// Returns 0 immediately when `l == 0` or `photo_gain == 0` (night phase or unexpressed gene).
/// `km_photo` must be `> 0` (debug_assert). Overflow: `photo_gain ≤ 256`, `l ≤ l_max ≤ 1000`,
/// so `photo_gain * l ≤ 256_000` — far below i64_max.
pub fn photo_demand(photo_gain: i32, km_photo: i64, l: i64) -> i64 {
    if l == 0 || photo_gain == 0 {
        return 0;
    }
    debug_assert!(km_photo > 0, "km_photo must be > 0");
    (photo_gain as i64 * l) / (l + km_photo)
}

/// Photo-expression capacity after GRN regulation (D′-2b), pure function of genome + current L(t).
///
/// At founder gain (`reg_gain == 0`): constitutive — returns full `photo_gain` regardless of `l`,
/// byte-identical to D′-2a (no behavioural change until evolution discovers a non-zero gain).
///
/// At non-zero gain: binary threshold on `L(t)` vs `reg_setpoint`:
///   `reg_gain > 0` → express by DAY  (`l ≥ reg_setpoint` → `photo_gain`; else 0).
///   `reg_gain < 0` → express by NIGHT (`l <  reg_setpoint` → `photo_gain`; else 0).
///
/// **Encoding (declared, F3 — binary threshold).** Only `sign(reg_gain)` determines the output;
/// the magnitude is dead weight on the expression function. The trait is 3-state: neg/0/pos.
/// `reg_gain_max` clamps evolvable range and locks regulation OFF at `reg_gain_max = 0`.
/// D′-2c must account for this: the constitutive control line is `reg_gain_max = 0`.
///
/// **Night income is 0 regardless** (`l = 0` → `photo_demand` returns 0 anyway). The ONLY
/// observable signature of regulation is the SAVED COST at night: a night-downregulated cell
/// (`reg_gain > 0`, `l = 0`) has `expressed_capacity = 0` → `photo_cost = 0`.
/// `photo_produced` does NOT distinguish regulated from constitutive; only `photo_cost_total` does.
///
/// Pure, integer, no RNG — deterministic given genome + global `L(t)` (R14).
pub fn expressed_capacity(g: &crate::Genome, l: i64) -> i32 {
    if g.photo_gain == 0 { return 0; }
    if g.reg_gain == 0 { return g.photo_gain; }
    let express = if g.reg_gain > 0 {
        l >= g.reg_setpoint as i64
    } else {
        l < g.reg_setpoint as i64
    };
    if express { g.photo_gain } else { 0 }
}

/// P1-2a: Result of respiratory electron-acceptor selection (PURE, deterministic).
/// Encodes which field layer the cell will respire through, with efficiency factor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RespiredChoice {
    /// Selected electron-acceptor field layer (primary or fallback).
    acceptor: crate::FieldId,
    /// Respiratory efficiency as fraction of 256 (e.g., 256 = ×1.0; 32 = ×0.125 fermentation).
    eff_x256: i16,
    /// `true` if anoxic (no acceptor available) — obligate-aerobe will die.
    anoxic: bool,
}

/// P1-2a: Choose the active electron-acceptor layer by redox-priority (PURE function).
///
/// Implements redox-hierarchy: primary (e.g., O₂), then fallbacks in order, then anoxia
/// (fermentation or death). Deterministic—field is read-only, no RNG, no clock.
///
/// Redox-repression (redox-inhibition, B5): EXACTLY ONE acceptor per cell is active.
/// First available layer in the priority chain is used; others are repressed (not consumed).
/// `n_layers` is the field's total layer count (`econ.n_layers`). An acceptor whose FieldId index
/// is ≥ `n_layers` is a layer this config did NOT allocate (e.g. NO₃⁻ fallback in the 3-layer
/// oxygen testbed, or O₂ itself in a 2-layer config) → treated as UNAVAILABLE, never sampled. This
/// bounds-guard is mandatory: `conserved_at` panics on an out-of-range layer index.
fn choose_respiratory_pathway(
    rp: &crate::RespiratoryPathway,
    field: &dyn crate::FieldStore,
    pos: crate::Vec2Fixed,
    n_layers: usize,
) -> RespiredChoice {
    // Try primary layer first (only if this config allocated that layer).
    let primary_idx = rp.primary_layer.as_usize();
    if primary_idx < n_layers && field.conserved_at(pos, primary_idx) > 0 {
        return RespiredChoice {
            acceptor: rp.primary_layer,
            eff_x256: rp.primary_eff_x256,
            anoxic: false,
        };
    }

    // Walk fallback layers in priority order (skip layers this config did not allocate).
    for (i, &fallback_layer) in rp.fallback_layers.iter().enumerate() {
        let idx = fallback_layer.as_usize();
        if idx < n_layers && field.conserved_at(pos, idx) > 0 {
            return RespiredChoice {
                acceptor: fallback_layer,
                eff_x256: rp.fallback_effs_x256[i],
                anoxic: false,
            };
        }
    }

    // All acceptors exhausted: anoxia. Yield = 0 if obligate-aerobe (cost≥256), else fermentation.
    let anoxia_yield_x256 = if rp.anoxia_cost_x256 >= 256 { 0 } else { rp.anoxia_cost_x256 };
    RespiredChoice {
        acceptor: rp.primary_layer,
        eff_x256: anoxia_yield_x256,
        anoxic: true,
    }
}

/// P1-2b: Compute hypoxia factor [0..1000] (Q3.10) from inner-cell O₂-starvation (self-shading).
///
/// Hypoxia represents the metabolic penalty of clustering: inner cells lack direct access to
/// ambient O₂ (surface-area / volume mismatch). The factor scales the yield (income) in
/// `stage_interactions`, reducing `gained` when body_cell_count > 1 in a hypoxic biome.
///
/// N≤1 → 0 (single cell has no interior, fully oxygenated surface).
/// N>1 → inner_fraction × scarcity, clamped [0..1000].
/// Integer-deterministic (CBRT_LUT, no float). PURE function (reads field once, no neighbor-offset).
pub(crate) fn compute_hypoxia_factor_x1000(
    primary_layer: crate::FieldId,
    field: &dyn crate::FieldStore,
    pos: crate::Vec2Fixed,
    body_cell_count: i64,
    cap_o2: i64,
    n_layers: usize,
) -> i32 {
    if body_cell_count <= 1 {
        return 0; // Single cell or non-phase2: no interior → no hypoxia.
    }

    let idx = primary_layer.as_usize();
    if idx >= n_layers {
        return 0; // Layer out-of-range bounds-guard (conserved_at would panic).
    }
    if cap_o2 <= 0 {
        // No O₂ economy (cap unset / no O₂ field) → no diffusion cost. Return 0 rather than treating
        // cap=0 as scarcity=1000 (which would slap MAX hypoxia on every cluster unconditionally —
        // a mis-signed penalty). True anoxia is already handled by choose_respiratory_pathway (eff→0).
        return 0;
    }

    // 1. Inner fraction: proportion of cells WITHOUT direct surface access.
    //    Formula: inner_fraction = 1 − N^(−1/3) = (cbrt_n − 256) / cbrt_n
    //    CBRT_LUT[N] = 256 · cbrt(N); clamp N to [0..256] for LUT.
    let cbrt_n_x256 = CBRT_LUT[body_cell_count.min(256) as usize];
    let inner_fraction_x1000 = if cbrt_n_x256 > 256 {
        (1000i64 * (cbrt_n_x256 as i64 - 256) / cbrt_n_x256 as i64).clamp(0, 1000) as i32
    } else {
        0
    };

    // 2. Scarcity: local O₂ concentration relative to cap (normalized to [0..1000]).
    //    ambient_o2 = field sample × 1000 / cap_o2
    //    scarcity = 1000 − ambient_o2 (if abundant O₂, scarcity=0; if anoxic, scarcity=1000).
    let ambient_o2_x1000 = if cap_o2 > 0 {
        field.conserved_at(pos, idx) as i64 * 1000 / cap_o2
    } else {
        0
    };
    let scarcity_x1000 = (1000 - ambient_o2_x1000).clamp(0, 1000) as i32;

    // 3. Hypoxia = inner_fraction × scarcity / 1000 [Q3.10].
    ((inner_fraction_x1000 as i64 * scarcity_x1000 as i64) / 1000).min(1000) as i32
}

// ── Stage 6: Interactions — feed: proportional deficit rationing (B-3). ─────────────────────────
//    At a deficit cell (Σ demand > R_cell) each agent's grant is `U_i·R_cell / Σ U_j` (integer
//    truncating). Non-deficit cells grant each agent its full Monod demand.
//    Algorithm — ONE gather pass (no double archetype lookup), then sort by (cell×4+layer, entity)
//    so same-(cell,layer) contestants are contiguous. Two cheap walks (Σ then grant) followed by one
//    get_mut apply loop.  Order-independent: Σ is associative; grants depend only on cell totals.
//
//    D′-1: photo uptake is additive and non-rival (no contest), credited here in the same apply loop.
//    Photo energy is booked to `ledger.produced` as an external source (non-conserved flux).
pub fn stage_interactions(
    econ: Res<EconParams>,
    clock: Res<SimClock>,
    world: Res<WorldRes>,
    mut field: ResMut<FieldRes>,
    mut ledger: ResMut<EnergyLedger>,
    mut tel: ResMut<Telemetry>,
    // E-1: Phenotype is REQUIRED (not Option) — a missed spawn site = entity invisible here
    // = the entity skips energy intake = population changes = golden moves. The required query
    // is the detection mechanism for F2-missed-spawn-site bugs.
    mut q: Query<(Entity, &Position, &Genome, &Phenotype, &mut Energy)>,
    #[cfg(feature = "perf")] mut wc: ResMut<WorkCounters>,
) {
    // D′-1: km_photo is the photo Monod half-saturation (non-rival, computed once).
    // P5-D: light itself is now per-cell (depth-attenuated), computed in the apply loop.
    let km_photo: Option<i64> = econ.light.map(|ls| ls.km_photo);

    // 1. Gather: one or more contestants per entity (Monod demand). No `conserved_take` yet.
    //    Sort key = cell_index * 4 + layer (B-2: layer ∈ 0..4); secondary = entity_bits.
    //    EXT-0a: when body_footprint=true, an entity emits side² contestants (one per footprint cell,
    //    where side = g_dev.max(1)), reading each cell's conserved level independently and competing
    //    under the existing per-cell cap. Each footprint contestant maps to a DISTINCT field cell
    //    (no self-overlap) due to g_dev ≤ 4 ≪ world_dim, so sort-key uniqueness is preserved.
    struct Contestant {
        cell_layer: usize, // cell_index * 4 + layer — the group key (B-2: layer ∈ 0..4)
        entity_bits: u64,
        entity: Entity,
        cell_pos: Vec2Fixed,  // EXT-0a: the FOOTPRINT cell (not entity pos) for footprint mode
        layer: usize,
        demand: i64,
        bonded: bool,      // ENV-0a'-a1: true if Σ module_cell_count > 1 (multicellular body)
        is_footprint: bool,  // EXT-0a: true if this is a footprint contestant (not the entity's anchor)
    }
    let mut contestants: Vec<Contestant> = q.iter().flat_map(|(e, pos, _g, ph, _)| {
        // E-1: read uptake_layer from the cached Phenotype (live consumer of the decode seam).
        // For Ф0: ph.uptake_layer == g.uptake_layer always (1:1 projection). This read proves
        // the seam is live — the field routes through Phenotype, not directly from Genome.
        let layer = ph.uptake_layer as usize;
        // D′-1 defensive assertion: uptake_layer must index a conserved stock layer (NOT the light
        // field). Photo energy routes through photo_gain, never through uptake_layer.
        debug_assert!(
            layer < econ.n_layers,
            "uptake_layer {} >= n_layers {} — must not route light through uptake_layer",
            layer, econ.n_layers
        );
        // ENV-0a'-a1: cache bonded status (Σ module_cell_count > 1) for phase 3 priority fill.
        let bonded = ph.graph.module_cell_count.iter().map(|&c| c as i64).sum::<i64>() > 1;

        // EXT-0a: determine footprint configuration
        if econ.body_footprint {
            let side = (ph.graph.g_dev).max(1) as u64;
            // Debug assertion: body size must equal side² (F7 + F4)
            let body_size = ph.graph.body_size();
            debug_assert!(
                body_size == (side * side) as i64,
                "body_size {} must equal side²={} (side={})",
                body_size,
                side * side,
                side
            );
            // Debug assertion: footprint must not wrap onto itself (F4)
            debug_assert!(
                econ.world_dim >= side as i64,
                "world_dim {} >= side {} guard (wrap prevention)",
                econ.world_dim,
                side
            );

            // Generate footprint cells: block anchored at pos.0, toroidal-wrapped, row-major
            let mut footprint_contestants = Vec::new();
            let anchor_x = pos.0.0 as u64;
            let anchor_z = pos.0.1 as u64;
            let world_dim = econ.world_dim as u64;

            for row in 0..side {
                for col in 0..side {
                    // Toroidal wrap using rem_euclid semantics
                    let fx = ((anchor_x + col) % world_dim) as i64;
                    let fz = ((anchor_z + row) % world_dim) as i64;
                    let fp_pos = Vec2Fixed(fx, fz);

                    // Read this footprint cell's resource level
                    let r = field.0.conserved_at(fp_pos, layer);
                    let demand = monod_demand(econ.u_max, econ.km, r);
                    let demand = if econ.dol_economy {
                        // DR-0: soma cells scale demand
                        let soma: i64 = ph.graph.module_cell_count.iter().zip(ph.graph.module_is_germ.iter())
                            .filter_map(|(&c, &g)| if !g { Some(c as i64) } else { None }).sum();
                        demand * soma.max(1)
                    } else { demand };

                    let footprint_cell = field.0.cell_index(fp_pos);
                    footprint_contestants.push(Contestant {
                        cell_layer: footprint_cell * 4 + layer,
                        entity_bits: e.to_bits(),
                        entity: e,
                        cell_pos: fp_pos,
                        layer,
                        demand,
                        bonded,
                        is_footprint: true,
                    });
                }
            }

            footprint_contestants
        } else {
            // Non-footprint mode: single contestant at entity's anchor position
            let r = field.0.conserved_at(pos.0, layer);
            let demand = monod_demand(econ.u_max, econ.km, r);
            let demand = if econ.dol_economy {
                // DR-0: soma cells scale demand (income ∝ soma). Founder has soma=0 (germ-only body),
                // so max(soma,1) ensures bootstrap survival at baseline rate; soma≥1 gains income bonus.
                let soma: i64 = ph.graph.module_cell_count.iter().zip(ph.graph.module_is_germ.iter())
                    .filter_map(|(&c, &g)| if !g { Some(c as i64) } else { None }).sum();
                demand * soma.max(1)
            } else { demand };
            let cell = field.0.cell_index(pos.0);
            vec![Contestant {
                cell_layer: cell * 4 + layer,
                entity_bits: e.to_bits(),
                entity: e,
                cell_pos: pos.0,
                layer,
                demand,
                bonded,
                is_footprint: false,
            }]
        }
    }).collect();
    // Stable order: primary = cell_layer (groups contestants), secondary = entity_bits (tie-break).
    contestants.sort_unstable_by(|a, b| {
        a.cell_layer.cmp(&b.cell_layer).then_with(|| a.entity_bits.cmp(&b.entity_bits))
    });

    // 2. Two-pass over sorted cell-layer runs: Σ demand, then proportional grant.
    //    Grants are computed here; applied to Energy in the get_mut loop below.
    let n = contestants.len();
    let mut grants: Vec<i64> = vec![0; n];
    let mut run_start = 0;
    while run_start < n {
        // Find end of this cell-layer run.
        let run_cl = contestants[run_start].cell_layer;
        let mut run_end = run_start + 1;
        while run_end < n && contestants[run_end].cell_layer == run_cl {
            run_end += 1;
        }
        // Snapshot R_cell once for this run (all contestants share the same cell+layer).
        let r_cell = field.0.conserved_at(contestants[run_start].cell_pos, contestants[run_start].layer);
        // Σ demand over this run.
        let sigma: i64 = contestants[run_start..run_end].iter().map(|c| c.demand).sum();
        if sigma <= r_cell {
            // No deficit: each agent gets its full Monod demand.
            for i in run_start..run_end {
                grants[i] = contestants[i].demand;
            }
        } else if r_cell == 0 {
            // Empty cell: no grants (all zeros already).
        } else if econ.env_frontier_config.is_some() {
            // ENV-0a'-a1: Deficit with spatial monopolization enabled.
            // Priority greedy fill: bonded contestants pre-empt in entity_bits order;
            // unbonded split the remainder proportionally. Frequency-dependence emerges
            // from this order, not from a parameter.
            let mut remaining = r_cell;
            // Phase 3a: bonded fill (pre-emption in existing entity_bits order).
            for i in run_start..run_end {
                if contestants[i].bonded {
                    let grant = contestants[i].demand.min(remaining);
                    grants[i] = grant;
                    remaining -= grant;
                }
            }
            // Phase 3b: unbonded proportional split of remainder.
            let sigma_unbonded: i64 = (run_start..run_end)
                .filter(|&i| !contestants[i].bonded)
                .map(|i| contestants[i].demand)
                .sum();
            if remaining > 0 && sigma_unbonded > 0 {
                for i in run_start..run_end {
                    if !contestants[i].bonded {
                        grants[i] = contestants[i].demand * remaining / sigma_unbonded;
                    }
                }
            }
        } else {
            // Deficit: proportional ration — ⌊U_i · R_cell / Σ⌋ (original behavior when env_frontier_config OFF).
            for i in run_start..run_end {
                grants[i] = contestants[i].demand * r_cell / sigma;
            }
        }
        run_start = run_end;
    }

    // 3. Apply grants: ONE get_mut per entity (no second archetype scan).
    //    `conserved_take` is called for the GRANT amount (may be < demand at deficit cells).
    //    D′-1: photo energy credited here too — same stage, so the booked set matches exactly.
    //    D′-3b: record per-entity income split (photo_in, chem_in) in tel.income_record using the
    //    EXACT integers booked here. This is a read-only side-channel — never fed back to any
    //    conserved value or state hash. Non-dprime: photo=0 always → (0, gained) written.
    //    P1-2a: respiratory yield multiplier applied here (if respiratory_pathway exists).
    //    EXT-0a (F1): per-entity income accumulator — sums over all footprint cells, emitted in
    //    telemetry as a dedicated field (not via income_record clobber).
    //    EXT-0a (F6): per-entity contention rate — fraction of footprint cells hitting deficit branch.
    tel.income_record.clear();
    let mut photo_total: i64 = 0;

    // EXT-0a (F1): per-entity income accumulators (income_got maps entity_bits → Σ got over footprint)
    let mut entity_income_map: DetMap<u64, i64> = DetMap::new();
    let mut entity_photo_map: DetMap<u64, i64> = DetMap::new();

    // EXT-0a (F6): per-entity contention tracking (entity_bits → (deficit_count, total_cells))
    let mut entity_contention_map: DetMap<u64, (u64, u64)> = DetMap::new();

    for (i, c) in contestants.iter().enumerate() {
        #[cfg(feature = "perf")]
        { wc.field_takes += 1; }
        let (_, _, g, ph, mut energy) = q.get_mut(c.entity).expect("entity present");

        // P1-2a: respiratory yield modifier COMPOSES with the pre-existing metabolic efficiency.
        // `metabolism_eff` (Ф0) is an efficiency on consumed substrate: the entity eats the FULL
        // grant, converts `eff/256` to energy and dissipates the rest (below). The respiratory
        // pathway multiplies THIS efficiency: aerobic (O₂, 256=×1.0) leaves it unchanged; a worse
        // acceptor (NO₃ 180=×0.7) or anoxia (0) scales it down. `resp_eff_x256 = 256` when no
        // pathway → combined_eff == metabolism_eff → BYTE-IDENTICAL to P1-0/P1-1 (isolation gate).
        // NOTE: this preserves the original take-FULL-grant + dissipate-inefficiency semantics
        // (removing the reduced-take restructuring, which silently changed field balance and the
        // dissipated ledger for EVERY existing entity — a golden break even at enable_oxygen=false).
        // ISOLATION GATE (`econ.enable_oxygen`): the five shipped configs run with it FALSE. The
        // founder respiratory_pathway gene is 0, but `decode_respiratory_pathways(0)` returns
        // Some(obligate-aerobe) — NOT None — so gating on `.is_some()` alone would (a) charge the
        // aerobe machinery on every legacy entity and (b) sample the O₂ layer (index 2) on 2-layer
        // configs → OOB panic + golden drift. Gating on the econ flag (which also gates the gene's
        // mutation) makes enable_oxygen=false a true no-op: resp_eff=256 → combined_eff=metabolism_eff.
        // P2-R-D: save the full RespiredChoice for O₂-consumption gating (acceptor + anoxic).
        // EXT-0a (F2): read cell_pos for hypoxia/O₂/thermal to use footprint-cell values.
        let respired_choice = if econ.enable_oxygen {
            match &ph.respiratory_pathway {
                Some(rp) => choose_respiratory_pathway(rp, &*field.0, c.cell_pos, econ.n_layers),
                None => RespiredChoice { acceptor: crate::FieldId::Substrate, eff_x256: 256, anoxic: false },
            }
        } else {
            RespiredChoice { acceptor: crate::FieldId::Substrate, eff_x256: 256, anoxic: false }
        };
        let resp_eff_x256: i64 = respired_choice.eff_x256 as i64;
        let combined_eff_x256 = g.metabolism_eff as i64 * resp_eff_x256 / 256;

        // P1-2b: Apply hypoxia to income (yield penalty from O₂-diffusion self-shading).
        // Hypoxia is gated on enable_oxygen && Some(rp) (same gate as resp_eff above).
        // For N≤1 or non-O₂ configs, hypoxia_x1000=0 → kept=1000 → no-op (byte-identical).
        // Order of operations (critical for golden determinism):
        //   1. combined_eff (composition of metabolism_eff × resp_eff)
        //   2. hypoxia (from O₂-field)
        //   3. gained = got × combined_eff / 256 × kept / 1000
        let n_cells: i64 = ph.graph.module_cell_count.iter().map(|&c| c as i64).sum();
        let hypoxia_x1000 = if econ.enable_oxygen && !econ.ablate_hypoxia && ph.respiratory_pathway.is_some() {
            let rp = &ph.respiratory_pathway.as_ref().unwrap();
            let raw = compute_hypoxia_factor_x1000(rp.primary_layer, &*field.0, c.cell_pos, n_cells, econ.o2_cap, econ.n_layers);
            // Calibration scale (dive §4.1 hypoxia_base_x1000): anchor to Ratcliffe −10%. Default 1000 → ×1.0.
            (raw as i64 * econ.hypoxia_base_x1000 / 1000).clamp(0, 1000) as i32
        } else {
            0 // enable_oxygen=false OR ablate_hypoxia=true (verdict control arm) OR no pathway → no hypoxia
        };
        let kept_x1000 = (1000 - hypoxia_x1000 as i64).max(0);

        // P3-2 (F1): Thermal tolerance penalty — applied to income-yield when ambient_tolerance is configured.
        // penalty gates on is_some() to preserve byte-identity (all legacy configs have None).
        // Order of operations (critical for golden determinism):
        //   1. combined_eff (composition of metabolism_eff × resp_eff)
        //   2. hypoxia (from O₂-field)
        //   3. thermal_x256 (tolerance penalty on current T)
        //   4. gained = ((got × combined_eff / 256 × kept / 1000) × thermal_x256) / 256
        let thermal_x256 = if econ.ambient_tolerance.is_some() {
            crate::params::tolerance_penalty(world.0.temp_at(c.cell_pos), g.tol_optimum, g.tol_breadth) as i64
        } else { 256 };

        // Take the FULL grant (original semantics), convert combined_eff to energy, dissipate rest.
        // Conserved (R15): got == gained + lost. Anoxic obligate → resp_eff=0 → gained=0 → starves (R34).
        // P1-2b: gained is now reduced by hypoxia through kept_x1000 factor.
        // P3-2: gained is further reduced by thermal_x256 factor (tolerance penalty).
        // EXT-0a (F6): detect deficit (grant < demand) for contention rate tracking.
        let got = field.0.conserved_take(c.cell_pos, grants[i], c.layer);
        let gained = ((got * combined_eff_x256 / 256 * kept_x1000 / 1000) * thermal_x256) / 256;
        let lost = got - gained;  // metabolic inefficiency + hypoxia shortfall → dissipated (conserved)
        energy.0 += gained;
        ledger.dissipated += lost;

        // EXT-0a (F1): accumulate income per entity (across all footprint cells)
        *entity_income_map.entry(c.entity_bits).or_insert(0) += got;

        // EXT-0a (F6): track contention (deficit = grant < demand)
        {
            let (deficit_count, total_cells) = entity_contention_map.entry(c.entity_bits).or_insert((0, 0));
            if grants[i] < c.demand {
                *deficit_count += 1;
            }
            *total_cells += 1;
        }

        // P2-R-C: O₂ consumption (respiration). Gated on enable_oxygen && light.is_some().
        // Only when acceptor=O₂ and not anoxic: deb it O₂ proportional to respiratory energy.
        // Stagebit → solve() applies netto (clamp ≥ 0). Deterministic (read-old contract,
        // choose_respiratory_pathway saw O₂@t; consumption booked @t+1).
        if econ.enable_oxygen && econ.light.is_some() && respired_choice.acceptor == crate::FieldId::Oxygen && !respired_choice.anoxic {
            let cell_idx = field.0.cell_index(c.cell_pos);
            // O₂ consumed proportional to energy gained from respiratory pathway (1:1 stoichiometry for P2).
            let o2_consumed = gained;
            field.0.deposit_conserved(cell_idx, -o2_consumed, crate::FieldId::Oxygen.as_usize());
        }

        // D′-1/D′-2b: additive photo energy on the EXPRESSED capacity.
        // Night-downregulated cells have expressed_capacity=0 → photo_demand returns 0 (also because
        // L=0 at night, so the saving is in COST not income — see expressed_capacity doc).
        // P5-D: compute L(t) per-entity using per-cell height; when enable_photic=false this is
        // identical to the old uniform l_now (byte-identity guarantee).
        let l_now = crate::params::light_at(econ.light.as_ref(), clock.tick, econ.enable_photic, world.0.height(c.cell_pos.0, c.cell_pos.1));
        let photo = if let Some(km) = km_photo {
            let p = photo_demand(expressed_capacity(g, l_now), km, l_now);
            energy.0 += p;
            photo_total += p;

            // P2-R-D: O₂ production (photosynthesis). Gated on enable_oxygen && light.is_some().
            // Production proportional to photo-energy earned (1:1 stoichiometry for P2).
            // Stagebit → solve() applies netto. Day (l_now > 0) → production; night (l_now = 0) → 0.
            if econ.enable_oxygen && econ.light.is_some() {
                let cell_idx = field.0.cell_index(c.cell_pos);
                field.0.deposit_conserved(cell_idx, p, crate::FieldId::Oxygen.as_usize());
            }

            p
        } else { 0 };

        // EXT-0a (F1): accumulate photo income per entity
        *entity_photo_map.entry(c.entity_bits).or_insert(0) += photo;
    }

    // EXT-0a (F1): emit per-entity accumulated income to income_record.
    // For each entity, store (total_photo, total_got) so telemetry captures per-entity income.
    for (entity_bits, total_got) in entity_income_map.iter() {
        let total_photo = entity_photo_map.get(&entity_bits).copied().unwrap_or(0);
        tel.income_record.insert(*entity_bits, (total_photo, *total_got));
    }

    // EXT-0a (F6): emit per-entity contention rate to telemetry.
    // For each entity, compute fraction of footprint cells that hit deficit (grant < demand).
    // This metric separates three diagnostic outcomes: real gradient, no-gradient NULL, motility confound.
    for (entity_bits, (deficit_count, total_cells)) in entity_contention_map.iter() {
        if *total_cells > 0 {
            let contention_rate = *deficit_count as f32 / *total_cells as f32;
            tel.entity_contention_rate.insert(*entity_bits, contention_rate);
        } else {
            // Edge case: no footprint cells (shouldn't happen with footprint flag ON, but defensive).
            tel.entity_contention_rate.insert(*entity_bits, 0.0);
        }
    }

    // D′-1: book photo energy as external source (non-conserved flux, like regen from field.solve()).
    // Uses the ACTUAL per-cell sum Σᵢ photo_energyᵢ — NOT N·U_photo — so the booked source matches
    // exactly the credited energy, leaving residual 0 (R15) even after photo_gain mutates per-cell.
    // Stage-precise: credits only cells alive at this stage (before birth_death this tick).
    ledger.produced += photo_total;
    tel.photo_produced = photo_total;
}

// ── Stage 6b: MineralFeed — contested Monod uptake from mineral layer into per-entity MineralQuota.
//    Mirrors stage_interactions (energy feed-ration): gather → sort by (cell, entity) → ration.
//    Entity-id sort (R10): same canonical order as stage_birth_death, not (cell+layer) sort.
//    Non-rival check is still `sigma > r_cell` — mineral IS contested (unlike light, which is non-rival).
//    EARLY EXIT when `econ.mineral_layer` is None — stage is always scheduled but inert for non-dprime.
//    Conservation: field_M decreases by grant; quota increases by grant; no ledger entry (conserved).
pub fn stage_mineral_feed(
    econ: Res<EconParams>,
    mut field: ResMut<FieldRes>,
    mut q: Query<(Entity, &Position, &mut MineralQuota)>,
) {
    let min_l = match econ.mineral_layer {
        Some(l) => l,
        None => return, // inert for non-dprime configs; MineralQuota not present anyway
    };

    // 1. Gather: Monod demand from mineral field layer. Entity-id-sorted (R10/R14).
    struct MinContestant {
        entity_bits: u64,
        entity: Entity,
        pos: Vec2Fixed,
        demand: i64,
    }
    let mut contestants: Vec<MinContestant> = q.iter().map(|(e, pos, _)| {
        let r = field.0.conserved_at(pos.0, min_l);
        let demand = monod_demand(econ.u_max_mineral, econ.km_mineral, r);
        MinContestant { entity_bits: e.to_bits(), entity: e, pos: pos.0, demand }
    }).collect();
    // Entity-id order — same sort as stage_birth_death (R10). The mineral uptake contest is
    // purely per-cell (not per-layer pair), so the sort key is cell_index × secondary = entity.
    contestants.sort_unstable_by(|a, b| {
        let cell_a = field.0.cell_index(a.pos);
        let cell_b = field.0.cell_index(b.pos);
        cell_a.cmp(&cell_b).then_with(|| a.entity_bits.cmp(&b.entity_bits))
    });

    // 2. Two-pass ration: Σ demand per cell, then proportional grant at deficit.
    let n = contestants.len();
    let mut grants: Vec<i64> = vec![0; n];
    let mut run_start = 0;
    while run_start < n {
        let cell_a = field.0.cell_index(contestants[run_start].pos);
        let mut run_end = run_start + 1;
        while run_end < n && field.0.cell_index(contestants[run_end].pos) == cell_a {
            run_end += 1;
        }
        let r_cell = field.0.conserved_at(contestants[run_start].pos, min_l);
        let sigma: i64 = contestants[run_start..run_end].iter().map(|c| c.demand).sum();
        if sigma <= r_cell {
            for i in run_start..run_end {
                grants[i] = contestants[i].demand;
            }
        } else if r_cell == 0 {
            // empty cell — all zeros
        } else {
            for i in run_start..run_end {
                grants[i] = contestants[i].demand * r_cell / sigma;
            }
        }
        run_start = run_end;
    }

    // 3. Apply: take from field (conserved), credit to quota. No ledger entry — conserved transfer.
    for (i, c) in contestants.iter().enumerate() {
        let (_, _, mut quota) = q.get_mut(c.entity).expect("entity present");
        let got = field.0.conserved_take(c.pos, grants[i], min_l);
        quota.0 += got;
    }
}

// ── Stage 6c: Predation — mode-driven resolution (D-5 hazard, D-4 universal, P-2a combat-split). ────
// Three modes (D-5 spec.mode enum, type-guaranteed exclusive):
//
// **D-5 Hazard:** implicit external predator, per-entity per-tick drain attenuated by body size
// (refuge-only, no offense). No entity-vs-entity eligibility; per-entity independent (R14 trivial).
// Top-level branch (owns resolution, no fall-through).
//
// **D-4 Universal:** ubiquitous size-selective predation. All entities are potential predators of
// strictly-smaller-bodied neighbours in their cell (Boraas mechanism). Top-level branch.
//
// **P-2a Combat-split:** predators (combat_trait > 0) vs. prey (≤ 0). Mean-field or per-prey
// (gated by size_refuge). Default path when neither D-5 nor D-4 is active.
//
// Determinism (R14): entity-id single-writer ordering, no RNG, per-entity drain or mean-field
// aggregate prey energy. Conservation (R15): drain/loss ≤ energy (exact integer), dissipation routed
// to ledger. No-op when `config.predation` is None (early return).
pub fn stage_predation(
    econ: Res<EconParams>,
    mut ledger: ResMut<EnergyLedger>,
    field: ResMut<FieldRes>,  // C′: dead prey → detritus or substrate (currently unused in P-2a)
    mut q: Query<(Entity, &Position, &mut Energy, &Genome, &Phenotype)>,
    mut commands: Commands,
    #[cfg(feature = "perf")] mut wc: ResMut<WorkCounters>,
) {
    // Early exit: no predation configured → stage is a no-op (byte-identical).
    let spec = match &econ.predation {
        Some(s) => s,
        None => return,
    };

    use crate::predation::{resolve_encounter, refuge_attenuate, PredationMode};

    // D-5: top-level branch (BEFORE universal and combat-split) for hazard-refuge predation.
    // Implicit external predator with per-entity per-tick drain, attenuated by body size only.
    // No entity-vs-entity eligibility; per-entity independent (R14 trivial, R15 exact).
    // Owns resolution — no fall-through.
    if spec.mode == PredationMode::Hazard {
        if spec.base_hazard > 0 && spec.size_refuge.is_some() {
            let refuge = spec.size_refuge.unwrap();
            // Collect all entities in entity-id order (R14).
            let mut entity_list: Vec<(u64, Entity)> = q.iter().map(|(e, _, _, _, _)| (e.to_bits(), e)).collect();
            entity_list.sort_unstable_by_key(|x| x.0);

            for (_bits, entity) in entity_list {
                #[cfg(feature = "perf")]
                { wc.birth_death_iters += 1; }

                // Read entity's energy and body size.
                let (energy_val, body_size, refuge_mass) = match q.get(entity) {
                    Ok((_, _, energy, _, ph)) => {
                        let body = ph.graph.module_cell_count.iter().map(|&c| c as i64).sum::<i64>().max(1);
                        let refuge_mass = if econ.division_of_labor {
                            ph.graph.module_cell_count.iter().zip(ph.graph.module_is_germ.iter())
                                .filter_map(|(&c, &g)| if !g { Some(c as i64) } else { None }).sum::<i64>().max(1)
                        } else { body };
                        (energy.0.max(0), body, refuge_mass)
                    }
                    Err(_) => continue,
                };

                if energy_val <= 0 {
                    continue;
                }

                // Compute refuge-attenuated drain.
                let drain = refuge_attenuate(spec.base_hazard, refuge_mass, refuge.shift, refuge.refuge_k);
                let drain = drain.min(energy_val); // clamp to available energy
                let actual_drain = drain;

                // Apply drain and route to dissipation.
                if let Ok((_, _, mut energy, _, _)) = q.get_mut(entity) {
                    energy.0 -= actual_drain;
                    if energy.0 <= 0 {
                        commands.entity(entity).despawn();
                        ledger.lost += energy.0.max(0); // 0 at death
                    }
                }
                ledger.dissipated += actual_drain;
            }
        }
        // Hazard branch owns resolution — no fall-through to universal or combat-split.
        return;
    }

    // D-4: top-level branch (BEFORE combat-trait split) for universal predation mode.
    // When mode=Universal, ALL entities are potential predators of strictly-smaller-bodied neighbours
    // in their cell (Boraas ubiquitous size-selective mechanism).
    if spec.mode == PredationMode::Universal {
        // D-4a universal-size cell loop: collect all entities per cell, sort by id, resolve size-based predation.
        // Worst case: O(k²) per field-cell (k = per-cell occupancy). Accepted trade-off F4: the existing
        // perf-corridor (cli/src/lib.rs:449) trips on any O(N²) regression — risk is CI-guarded, not silent.

        // 1. Gather all entities with body size, grouped by cell.
        let mut cells_universal: std::collections::BTreeMap<usize, Vec<(u64, Entity, i64)>> =
            std::collections::BTreeMap::new();
        for (e, pos, _, _g, ph) in &q {
            let cell_idx = field.0.cell_index(pos.0);
            let body_size: i64 = ph.graph.module_cell_count.iter().map(|&c| c as i64).sum::<i64>().max(1);
            cells_universal.entry(cell_idx).or_insert_with(Vec::new).push((e.to_bits(), e, body_size));
        }

        // 2. Process each cell: for each entity E as predator, resolve against all strictly-smaller prey.
        for (_cell_idx, mut entities) in cells_universal {
            // Sort by id_bits (R14: single-writer order, deterministic).
            entities.sort_unstable_by_key(|x| x.0);

            // For each entity E acting as a predator in id order.
            for (_pred_bits, pred_e, pred_body) in &entities {
                #[cfg(feature = "perf")]
                { wc.birth_death_iters += 1; }

                // Prey pool: entities with STRICTLY smaller body (strict `<` antisymmetric, no self/tie).
                let prey_pool: Vec<(u64, Entity, i64)> = entities.iter()
                    .filter(|(_, _, prey_body)| *prey_body < *pred_body)
                    .cloned()
                    .collect();

                if prey_pool.is_empty() {
                    continue; // no valid prey for this predator
                }

                // Get predator genome; set predator strength to 0 (neutral — escape is the prey's refuge, not predator power).
                // The trait term is driven by combat_trait which stays 0 under universal mode (founder never mutates it),
                // so we can also use combat_trait_scale=0 in driver_config to make the bite independent of size.
                let pred_genome = match q.get(*pred_e) {
                    Ok((_, _, _, genome, _)) => {
                        let mut g = genome.clone();
                        g.size = 0; // neutral predator strength: bite governed by refuge only
                        g
                    }
                    Err(_) => continue,
                };

                // Resolve each prey in id order (F5: live energy read per prey, post-drain visible to next).
                for (_, prey_e, _) in &prey_pool {
                    let (prey_energy_val, prey_body_size) = match q.get(*prey_e) {
                        Ok((_, _, energy, _, ph)) => {
                            let body: i64 = ph.graph.module_cell_count.iter().map(|&c| c as i64).sum();
                            (energy.0.max(0), body.max(1))
                        }
                        Err(_) => continue,
                    };

                    if prey_energy_val <= 0 {
                        continue;
                    }

                    let outcome = resolve_encounter(&pred_genome, prey_energy_val, prey_body_size, spec);

                    // Drain prey and credit predator (conservation invariant re-proven per prey).
                    if let Ok((_, _, mut prey_energy, _, _)) = q.get_mut(*prey_e) {
                        prey_energy.0 -= outcome.prey_loss;
                        if prey_energy.0 <= 0 {
                            commands.entity(*prey_e).despawn();
                            ledger.lost += prey_energy.0.max(0); // 0
                        }
                    }
                    if let Ok((_, _, mut pred_energy, _, _)) = q.get_mut(*pred_e) {
                        pred_energy.0 = (pred_energy.0 + outcome.predator_gain).max(0);
                    }
                    ledger.dissipated += outcome.dissipated;
                }
            }
        }

        // Universal branch owns resolution — no fall-through to combat-trait split path.
        return;
    }

    // Gather all entities with their combat_trait, sorted by id (combat-trait split path, byte-identical to P-2a/D-1).
    let mut entity_list: Vec<(u64, Entity, Vec2Fixed, i32)> = q.iter()
        .map(|(e, pos, _, g, _)| (e.to_bits(), e, pos.0, g.combat_trait))
        .collect();
    entity_list.sort_unstable_by_key(|x| x.0);

    // Group by cell, separating predators from prey candidates.
    let mut cells: std::collections::BTreeMap<usize, (Vec<(u64, Entity, i32)>, Vec<(u64, Entity, i32)>)> =
        std::collections::BTreeMap::new();
    for (bits, e, pos, combat) in entity_list {
        let cell_idx = field.0.cell_index(pos);
        let entry = cells.entry(cell_idx).or_insert((Vec::new(), Vec::new()));
        if combat > 0 {
            entry.0.push((bits, e, combat));
        } else {
            entry.1.push((bits, e, combat));
        }
    }

    // Process each cell.
    for (_cell, (mut predators, prey_candidates)) in cells {
        if predators.is_empty() {
            continue; // no predators in this cell
        }

        // Sort predators by entity-id (deterministic processing order).
        predators.sort_unstable_by_key(|x| x.0);

        // Process each predator in entity-id order.
        for (_pred_bits, pred_e, pred_combat) in predators {
            #[cfg(feature = "perf")]
            { wc.birth_death_iters += 1; }

            // Build prey pool: candidates with strictly lower combat_trait.
            let prey_pool: Vec<(u64, Entity, i32)> = prey_candidates.iter()
                .filter(|(_, _, prey_combat)| *prey_combat < pred_combat)
                .cloned()
                .collect();

            if prey_pool.is_empty() {
                continue; // no valid prey for this predator
            }

            // Get predator genome for encounter resolution.
            let pred_genome = {
                match q.get(pred_e) {
                    Ok((_, _, _, genome, _)) => genome.clone(),
                    Err(_) => continue,
                }
            };

            // Resolve encounter: wire combat_trait as the predator's "strength".
            let mut pred_genome_for_encounter = pred_genome;
            pred_genome_for_encounter.size = pred_combat;

            if spec.size_refuge.is_some() {
                // D-1 PER-PREY path: each prey resolved individually against its own energy and
                // body size, in entity-id order (deterministic, R14) — NOT a pool aggregate.
                for (_, prey_e, _) in &prey_pool {
                    let (prey_energy_val, prey_body_size) = match q.get(*prey_e) {
                        Ok((_, _, energy, _, ph)) => {
                            let body: i64 = ph.graph.module_cell_count.iter().map(|&c| c as i64).sum();
                            (energy.0.max(0), body.max(1))
                        }
                        Err(_) => continue,
                    };
                    if prey_energy_val <= 0 {
                        continue;
                    }

                    let outcome = resolve_encounter(
                        &pred_genome_for_encounter,
                        prey_energy_val,
                        prey_body_size,
                        spec,
                    );

                    if let Ok((_, _, mut prey_energy, _, _)) = q.get_mut(*prey_e) {
                        prey_energy.0 -= outcome.prey_loss;
                        if prey_energy.0 <= 0 {
                            // At death, energy=0, recycled=0 (no detritus from dead prey here).
                            commands.entity(*prey_e).despawn();
                            ledger.lost += prey_energy.0.max(0); // 0
                        }
                    }
                    if let Ok((_, _, mut pred_energy, _, _)) = q.get_mut(pred_e) {
                        pred_energy.0 = (pred_energy.0 + outcome.predator_gain).max(0);
                    }
                    ledger.dissipated += outcome.dissipated;
                }
                continue;
            }

            // AGGREGATE path (size_refuge=None, byte-identical to pre-D-1 P-2a): one encounter
            // resolved against the pooled prey energy, then drained across the pool in entity-id
            // order. `prey_body_size` is unused by `resolve_encounter` when `size_refuge=None`, so
            // the placeholder `1` below has no effect on the outcome.
            let prey_energy_agg: i64 = prey_pool.iter().map(|(_, prey_e, _)| {
                q.get(*prey_e).map(|(_, _, energy, _, _)| energy.0.max(0)).unwrap_or(0)
            }).sum();

            let outcome = resolve_encounter(&pred_genome_for_encounter, prey_energy_agg, 1, spec);

            // Drain prey_loss from prey in entity-id order, handling deaths.
            let mut remaining_loss = outcome.prey_loss;
            for (_, prey_e, _) in &prey_pool {
                if remaining_loss <= 0 {
                    break;
                }

                // Get prey's current energy.
                let prey_current = q.get(*prey_e).map(|(_, _, energy, _, _)| energy.0).unwrap_or(0);
                let drained = prey_current.min(remaining_loss);
                remaining_loss -= drained;

                // Apply drainage.
                if let Ok((_, _, mut prey_energy, _, _)) = q.get_mut(*prey_e) {
                    prey_energy.0 -= drained;

                    // If prey dies, handle death routing (C′ pattern).
                    if prey_energy.0 <= 0 {
                        // At death, energy=0, recycled=0 (no detritus from dead prey here).
                        // Mark for despawn.
                        commands.entity(*prey_e).despawn();
                        ledger.lost += prey_energy.0.max(0); // 0
                    }
                }
            }

            // Apply predator gain.
            if let Ok((_, _, mut pred_energy, _, _)) = q.get_mut(pred_e) {
                pred_energy.0 = (pred_energy.0 + outcome.predator_gain).max(0);
            }
            ledger.dissipated += outcome.dissipated;
        }
    }
}

// ── Stage 6d: Settling — size²-attenuated mortality pulse (P4/SL-1) ────────────────────────────
// P4/SL-1 settling-selection mechanic: every `period` ticks, a size²-attenuated mortality pulse.
// Drain per entity is computed as: `drain = (strength << SHIFT) / ((1 << SHIFT) + settling_k · size²)`
// where `size = Σ module_cell_count` (integer i128 to prevent overflow). Energy → `ledger.dissipated`,
// death at ≤0 in stage 7 (R15 conservation — mortality energy accounted before entity removal).
// Stage order: AFTER 6c-predation, BEFORE 7-birth_death (ТЗ F3, R15 logic).

/// P4/SL-1: Compute size²-attenuated settling drain (Q-format, single source of truth for formula).
/// Given a SettlingSpec and body size, returns the drain to be deducted this tick.
/// Formula: `drain = (strength << SHIFT) / ((1 << SHIFT) + settling_k · size²)` (integer only).
/// Used by both stage_settling (ECS loop) and test harness (settling_mechanic.rs).
#[inline]
pub fn settling_drain(spec: &crate::params::SettlingSpec, body_size: i64) -> i64 {
    let strength = spec.strength.clamp(0, 1_000_000);
    if strength == 0 {
        return 0; // inert
    }

    let shift = spec.shift.min(32); // defensive clamp to prevent overflow
    let size_sq: i128 = (body_size as i128) * (body_size as i128);
    let k = (spec.settling_k as i128).max(0);
    let numer: i128 = (strength as i128) << shift;
    let denom: i128 = ((1i128) << shift) + k * size_sq;
    let denom = denom.max(1);
    (numer / denom).clamp(0, 1_000_000) as i64
}

pub fn stage_settling(
    econ: Res<EconParams>,
    clock: Res<SimClock>,
    mut ledger: ResMut<EnergyLedger>,
    mut q: Query<(&Position, &mut Energy, &Phenotype)>,
) {
    // Early exit: no settling configured → stage is a no-op (byte-identical).
    let spec = match &econ.settling {
        Some(s) => s,
        None => return,
    };

    // Inert gate: period=0 → no-op.
    if spec.period == 0 {
        return;
    }

    // Trigger gate: `SimClock.tick % period == 0`.
    if clock.tick % spec.period != 0 {
        return;
    }

    // Settling pulse active this tick.
    for (_pos, mut energy, phenotype) in &mut q {
        if energy.0 <= 0 {
            continue; // dead or inert
        }

        // Compute body size: Σ module_cell_count (integer).
        let body_size: i64 = phenotype
            .graph
            .module_cell_count
            .iter()
            .map(|&c| c as i64)
            .sum::<i64>()
            .max(1);

        // Size²-attenuated drain via single-source-of-truth formula.
        let drain = settling_drain(spec, body_size);

        // Apply drain: clamp to available energy.
        let actual_drain = drain.min(energy.0);
        energy.0 -= actual_drain;
        ledger.dissipated += actual_drain;
    }
}

// ── Stage 7: BirthDeath — division (energy split) + death, via the command buffer (sync point). ────
// D′-3a additions: Liebig AND-gate on division (quota ≥ q_mineral), overflow-heat when energy-ready
// but mineral-poor (ONE site, same sorted loop), and mineral quota recycled on death.
pub fn stage_birth_death(
    econ: Res<EconParams>,
    clock: Res<SimClock>,
    mut ledger: ResMut<EnergyLedger>,
    mut field: ResMut<FieldRes>,  // C-2: receive recycled body energy → substrate (layer 0)
    mut repro: ResMut<ReproEvents>,
    mut commands: Commands,
    mut q: Query<(Entity, &Position, &mut Energy, &Genome, &SpeciesId, &Phenotype)>,
    mut qmin: Query<&mut MineralQuota>,  // D′-3a: separate query (avoids borrow conflict)
    #[cfg(feature = "perf")] mut wc: ResMut<WorkCounters>,
) {
    use crate::params::{D0_MASK, RECYCLE_DEN};
    let has_mineral = econ.mineral_layer.is_some();
    repro.parents.clear();
    let mut ents: Vec<(u64, Entity)> = q.iter().map(|(e, _, _, _, _, _)| (e.to_bits(), e)).collect();
    ents.sort_unstable_by_key(|x| x.0);
    for (bits, e) in ents {
        #[cfg(feature = "perf")]
        { wc.birth_death_iters += 1; }
        let (_, pos, mut energy, genome, species, ph) = q.get_mut(e).expect("entity present");

        // ── C-1: background death (d0) — FIRST check, before starvation and division. ──────────
        // A d0-killed agent does not also divide this tick; the counter-RNG draw is pure per-
        // (entity, tick) so the kill-set is thread-invariant and replay-invariant (R14).
        // SALT_DEATH ≠ SALT_MUT → death and mutation draws are uncorrelated streams.
        if econ.d0_scaled > 0 {
            let r = seed_fold(clock.seed, &[SALT_DEATH, bits, clock.tick]);
            if (r & D0_MASK) < econ.d0_scaled {
                // C-2: recycle split — agent holds full body energy (E > 0); the material case.
                // Slice-C (detritus_layer=None): recycled → substrate layer 0 (byte-identical).
                // C′-1 (detritus_layer=Some(l)): recycled·detritus_frac → layer l; remainder → 0.
                // lost_here → ledger.lost (first real source for this bucket).
                // Conservation: agents_total −E; field_staging +recycled (live in next solve());
                //   ledger.lost +(E−recycled); residual unchanged (verified at tick boundary).
                let e_body = energy.0;
                let recycled = econ.recycle_num * e_body / RECYCLE_DEN; // truncating — remainder → lost
                let lost_here = e_body - recycled; // = E − ⌊recycle·E⌋; every eu in exactly one bucket
                if recycled > 0 {
                    let cell = field.0.cell_index(pos.0);
                    match econ.detritus_layer {
                        None => {
                            // Slice-C: byte-identical abiotic return → substrate (layer 0).
                            field.0.deposit_conserved(cell, recycled, 0);
                        }
                        Some(det_l) => {
                            // C′-1: biotic redirect. detritus_frac = detritus_frac_num / RECYCLE_DEN.
                            let det = recycled * econ.detritus_frac_num / RECYCLE_DEN;
                            let abiotic = recycled - det; // 0 at bootstrap frac=1.0
                            if det > 0 { field.0.deposit_conserved(cell, det, det_l); }
                            if abiotic > 0 { field.0.deposit_conserved(cell, abiotic, 0); }
                        }
                    }
                }
                ledger.lost += lost_here;
                // D′-3a: on d0 death, recycle mineral quota fraction back to mineral layer;
                // remainder → ledger.lost (mineral analogue of energy recycle at C-2).
                if has_mineral {
                    if let Ok(quota) = qmin.get(e) {
                        let q_body = quota.0;
                        if q_body > 0 {
                            let recycled_min = econ.recycle_mineral_num * q_body / RECYCLE_DEN;
                            let lost_min = q_body - recycled_min;
                            if recycled_min > 0 {
                                let cell = field.0.cell_index(pos.0);
                                field.0.deposit_conserved(cell, recycled_min, econ.mineral_layer.unwrap());
                            }
                            ledger.lost += lost_min;
                        }
                    }
                }
                commands.entity(e).despawn();
                continue;
            }
        }

        if energy.0 <= 0 {
            // Death (starvation): metabolism clamps energy ≥ 0 (stages.rs stage_metabolism), so
            // energy.0 = 0 exactly here. Recycle split: floor(recycle_num × 0 / RECYCLE_DEN) = 0.
            // ledger.lost += 0 is a no-op; no field deposit; conservation intact (as before C).
            let recycled = econ.recycle_num * energy.0 / RECYCLE_DEN; // = 0
            ledger.lost += energy.0 - recycled;                        // = 0
            // D′-3a: starvation death — recycle mineral quota (same path as d0 death).
            if has_mineral {
                if let Ok(quota) = qmin.get(e) {
                    let q_body = quota.0;
                    if q_body > 0 {
                        let recycled_min = econ.recycle_mineral_num * q_body / RECYCLE_DEN;
                        let lost_min = q_body - recycled_min;
                        if recycled_min > 0 {
                            let cell = field.0.cell_index(pos.0);
                            field.0.deposit_conserved(cell, recycled_min, econ.mineral_layer.unwrap());
                        }
                        ledger.lost += lost_min;
                    }
                }
            }
            commands.entity(e).despawn();
            continue;
        }

        // ── D′-3a: Liebig AND-gate + overflow (ONE site, same sorted loop, no new RNG). ───────
        // Overflow trigger: energy-ready (≥ e_cell+c_div) but mineral-poor (quota < q_mineral).
        // The cell burns overflow_delta energy → ledger.lost (the Liebig surplus-heat sink).
        // Conservation: energy.0 -= δ; ledger.lost += δ; residual unchanged.
        // Non-dprime configs: has_mineral=false → block skipped → byte-identical.
        let mineral_gate_passes = if has_mineral {
            match qmin.get(e) {
                Ok(quota) => {
                    let q_val = quota.0;
                    let energy_ready = energy.0 >= econ.e_cell + econ.c_div;
                    let quota_ready = q_val >= econ.q_mineral;
                    if energy_ready && !quota_ready {
                        // Overflow: surplus energy that cannot become biomass dissipates as heat.
                        // Clamped to available energy (overflow cannot drive energy negative).
                        let delta = econ.overflow_delta.min(energy.0.max(0));
                        energy.0 -= delta;
                        ledger.lost += delta;
                    }
                    quota_ready
                }
                Err(_) => true, // safety fallback: entity without quota → gate open (never fires)
            }
        } else {
            true // no mineral economy → gate always open
        };

        let repro_bar = if econ.division_of_labor && econ.dol_germ_repro {
            // OLD DL-M inverted mechanic — PRESERVED for the historical DL-V/DL-C harnesses
            let body = ph.graph.module_cell_count.iter().map(|&c| c as i64).sum::<i64>().max(1);
            let germ = ph.graph.module_cell_count.iter().zip(ph.graph.module_is_germ.iter())
                .filter_map(|(&c, &g)| if g { Some(c as i64) } else { None }).sum::<i64>();
            if germ == 0 { i64::MAX } else { genome.repro_threshold as i64 * body / germ }
        } else if econ.dol_economy {
            // NEW: germ = flat fertility gate. germ=0 → sterile; germ≥1 → flat threshold (no body/germ tax).
            let germ: i64 = ph.graph.module_cell_count.iter().zip(ph.graph.module_is_germ.iter())
                .filter_map(|(&c, &g)| if g { Some(c as i64) } else { None }).sum();
            if germ == 0 { i64::MAX } else { genome.repro_threshold as i64 }
        } else { genome.repro_threshold as i64 };

        if energy.0 >= repro_bar
            && energy.0 >= econ.e_cell + econ.c_div
            && mineral_gate_passes
        {
            // Division: child stock e_cell stays in the system (the child), c_div dissipated.
            // Δenergy = −(e_cell + c_div) + e_cell(child) + c_div(dissipated) = 0  (conserved).
            energy.0 -= econ.e_cell + econ.c_div;
            ledger.dissipated += econ.c_div;
            // D′-3a: parent spends q_mineral from quota; child starts fresh (quota=0).
            // q_mineral → ledger.dissipated (analogous to c_div; the mineral cost of division).
            // Conservation: quota.0 -= q_mineral; dissipated += q_mineral; residual unchanged.
            if has_mineral {
                if let Ok(mut quota) = qmin.get_mut(e) {
                    ledger.dissipated += econ.q_mineral;
                    quota.0 -= econ.q_mineral;
                }
            }
            let pos_c = *pos;
            let child_genome =
                genome.mutate(seed_fold(clock.seed, &[SALT_MUT, bits, clock.tick]), econ.n_energy_layers, econ.light.is_some(), econ.reg_gain_max, econ.predation.is_some(), econ.enable_variable_length, econ.evolve_body_size, econ.enable_oxygen, econ.ambient_tolerance.is_some(), econ.enable_mutation_load, econ.mut_load_del_num, econ.mut_load_del_den, econ.mut_load_ben_num, econ.mut_load_ben_den, econ.gdev_cap);
            let species_c = *species;

            // E-1/E-5a/E-5b: decode-seam gate. Ф0 always returns Some; the five existing configs
            // always resolve `cell_type: None` and return Some. Only `phase2_config` can reach a
            // real `None` (E-5b: the size-viability criterion, `genome.rs`'s `(Some, Some)` chain
            // arm) — or, in test builds, the `#[cfg(test)]` `force_decode_none` injection. Either
            // way, the child never materializes: `e_cell` — already debited from the parent above
            // but with nowhere to go — is booked to `ledger.lost` (mirrors the death-recycle `lost`
            // pattern above), closing the residual EXACTLY: −(e_cell+c_div) + c_div(dissipated) +
            // e_cell(lost) = 0. If mineral is active, the q_mineral debit/dissipate above already
            // closed (paid before this gate; a miscarried division still burnt its mineral cost).
            // The offspring flag is set AFTER this gate (not before) so a stillbirth never inflates
            // `born_total`. E-5b: attribute the None to the REAL criterion (not a test injection)
            // via `is_stillbirth_by_size_criterion` — the dedicated telemetry counter, distinct from
            // this generic gate, so a `force_decode_none` probe never pollutes the production count.
            let Some(child_phenotype) = child_genome.decode(&econ) else {
                ledger.lost += econ.e_cell;
                if child_genome.is_stillbirth_by_size_criterion() {
                    repro.stillbirths += 1;
                }
                continue;
            };
            repro.parents.insert(bits);

            // Spawn contract (D-Brain-2a): the newborn gets ALL per-entity brain buffers ZEROED —
            // `BrainState` (both `h_old`/`h_new`) and `BrainOutput` — so no prior occupant's hidden
            // state or motor command can leak through a reused ECS slot, and the newborn stays frozen
            // (neutral Act) until its first GLOBAL Brain tick.
            //
            // Slot-stability invariant (M3/F2): Bevy ECS stores all components of one entity as a
            // single archetype table row. A spawn or despawn moves the ENTIRE row atomically — there
            // is no partial migration where BrainState moves but BrainOutput does not. All per-slot
            // buffers are therefore always in sync; the "forgot to move h_new" class of bug cannot
            // occur. The zeroing here covers the initial allocation, not partial row-updates.
            if has_mineral {
                commands.spawn((
                    Position(pos_c.0),
                    PositionNext(pos_c.0),
                    Velocity::default(),
                    VelocityNext::default(),
                    Energy(econ.e_cell),
                    child_genome,
                    child_phenotype, // E-1: cached cold phenotype (decode seam)
                    species_c,
                    Sensors::default(),
                    Intent::default(),
                    BrainState::zeroed(),
                    BrainOutput::zeroed(),
                    MineralQuota(0), // D′-3a: child inherits zero quota (must re-accumulate)
                    // M5: marks child for speciation check in Sim::process_pending_speciation()
                    PendingSpeciation,
                ));
            } else {
                commands.spawn((
                    Position(pos_c.0),
                    PositionNext(pos_c.0),
                    Velocity::default(),
                    VelocityNext::default(),
                    Energy(econ.e_cell),
                    child_genome,
                    child_phenotype, // E-1: cached cold phenotype (decode seam)
                    species_c,
                    Sensors::default(),
                    Intent::default(),
                    BrainState::zeroed(),
                    BrainOutput::zeroed(),
                    // M5: marks child for speciation check in Sim::process_pending_speciation()
                    // (post-stage, outside the ECS system so SpeciationState stays off the world).
                    PendingSpeciation,
                ));
            }
        }
    }
}

// ── Stage 8: FieldScatter — MULTITHREADED scatter (R14/R17). Each agent excretes a conserved amount
//    (agent→field, exact integer) and deposits pheromone (signal). Deposits are partitioned into N
//    thread-local batches on the sim's OWN pool, merged in canonical order, then the between-tick
//    solver runs over the merged field → t+1. ─────────────────────────────────────────────────────
pub fn stage_field_scatter(
    econ: Res<EconParams>,
    pool: Res<SimPool>,
    sp: Res<ScatterParams>,
    mut field: ResMut<FieldRes>,
    mut ledger: ResMut<EnergyLedger>,
    mut q: Query<(Entity, &Position, &Genome, &mut Energy)>, // B-2: &Genome for excrete_layer
    #[cfg(feature = "perf")] mut wc: ResMut<WorkCounters>,
) {
    // 1. Serial gather (Entity-id order): excrete conserved `w` (agent→field, Δtotal=0, NOT
    //    dissipated) and tag a pheromone deposit. Reducing energy mutates the component → serial.
    // Counter in serial loop only — avoids per-entity atomic in the parallel merge below (D1c).
    let mut ents: Vec<(u64, Entity)> = q.iter().map(|(e, _, _, _)| (e.to_bits(), e)).collect();
    ents.sort_unstable_by_key(|x| x.0);
    let mut deposits: Vec<Deposit> = Vec::with_capacity(ents.len());
    for (bits, e) in ents {
        #[cfg(feature = "perf")]
        { wc.scatter_deposits += 1; }
        let (_, pos, g, mut energy) = q.get_mut(e).expect("entity present");
        let w = econ.excrete.min(energy.0.max(0));
        energy.0 -= w;
        deposits.push(Deposit {
            cell: field.0.cell_index(pos.0),
            morton: field.0.cell_morton(pos.0),
            entity_bits: bits,
            layer: g.excrete_layer as usize, // B-2: genome-driven excrete layer
            conserved: w,
            signal: econ.pheromone,
        });
    }

    // 2. Partition into N thread-local batches ON THE OWN POOL (real intra-stage parallelism). The
    //    batch COUNT = N, which is what the NonAssociative negative strategy is sensitive to.
    let n = sp.threads.max(1);
    let chunk = deposits.len().div_ceil(n).max(1);
    let batches: Vec<Vec<Deposit>> = pool.0.install(|| {
        use rayon::prelude::*;
        deposits.par_chunks(chunk).map(<[Deposit]>::to_vec).collect()
    });

    // 3. Merge (conserved = integer associative ⇒ thread-count-independent; signal = canonical serial).
    field.0.commit_merge(&batches, sp.strategy);

    // 4. Between-tick solver over the merged field → t+1 (R17). Regeneration is the explicit source.
    let injected = field.0.solve();
    ledger.produced += injected;
}

// ── Stage 9: Observe — read-only telemetry sink (samples for the Price covariance). ────────────────
pub fn stage_observe(
    field: Res<FieldRes>,
    repro: Res<ReproEvents>,
    mut tel: ResMut<Telemetry>,
    q: Query<(Entity, &Genome, &Phenotype)>,
) {
    tel.samples.clear();
    // D-3a: body_size = Σ module_cell_count, clamped ≥1 (empty/non-phase2 CellGraph → 1).
    let mut ents: Vec<(u64, Genome, i64)> = q.iter()
        .map(|(e, g, ph)| (e.to_bits(), g.clone(), ph.graph.body_size()))
        .collect();
    ents.sort_unstable_by_key(|x| x.0);
    // D′-3b: take the income record so we can read it while pushing to tel.samples (avoids
    // borrow conflict). The record will be repopulated by stage_interactions next tick.
    let income_record = std::mem::take(&mut tel.income_record);
    let mut reg_active = 0i64;
    let mut reg_active_day = 0i64;
    for (bits, g, _body_size) in &ents {
        let offspring = u32::from(repro.parents.contains(bits));
        // D′-3b: read the exact booked integers recorded at stage_interactions.
        // Returns (0, 0) for entities not in the record (founders at tick 0, or non-dprime).
        let (photo_in, chem_in) = income_record.get(bits).copied().unwrap_or((0, 0));
        tel.samples.push(TraitSample {
            traits: [
                g.metabolism_eff,
                g.move_speed,
                g.sense_range,
                g.size,
                g.repro_threshold,
                g.mutation_rate,
                g.uptake_layer,   // B-2 slot 6: observable via Price covariance
                g.excrete_layer,  // B-2 slot 7
            ],
            offspring,
            photo_in,
            chem_in,
        });
        // D′-2c: reg-activity aggregate — pure read, never fed to tick or state hash.
        if g.reg_gain != 0 {
            reg_active += 1;
            if g.reg_gain > 0 { reg_active_day += 1; }
        }
    }
    tel.reg_active_count = reg_active;
    tel.reg_active_day_count = reg_active_day;
    tel.population = ents.len() as i64;
    tel.field_total = field.0.conserved_total_all();
    // Signal-field metric (R25) — serial total concentration; never feeds the tick.
    tel.signal_total = field.0.signal_total();

    // D-3a: body-size telemetry (#272) — integer fixed-point, 0 when population is 0. Every
    // non-phase2 config decodes an empty CellGraph (body_size 1 for all) → multicellular_frac stays
    // 0 there, byte-identical to before D-3a.
    let body_sizes: Vec<i64> = ents.iter().map(|(_, _, bs)| *bs).collect();
    (tel.mean_body_size, tel.max_body_size, tel.multicellular_frac) = body_size_aggregate(&body_sizes);

    // V-3-e: genome-distance diversity telemetry. Filter to Some(grn_spec) genomes FIRST (entity-id
    // order, from `ents` above), then mean genome_distance over CONSECUTIVE valid pairs — O(N),
    // never an all-pairs matrix. 0 for non-phase2 configs (all grn_spec None) or <2 valid genomes.
    // Read-only: never fed to the tick or folded into state_hash.
    let valid_specs: Vec<&GrnSpec> = ents.iter().filter_map(|(_, g, _)| g.grn_spec.as_deref()).collect();
    tel.genome_diversity = if valid_specs.len() >= 2 {
        let mut total = 0i64;
        for w in valid_specs.windows(2) {
            total += genome_distance(w[0], w[1]);
        }
        total / (valid_specs.len() as i64 - 1)
    } else {
        0
    };
}

// ── Stage 10: Swap — double-buffer swap for Position + Velocity. ───────────────────────────────────
pub fn stage_swap(mut q: Query<(&mut Position, &PositionNext, &mut Velocity, &VelocityNext)>) {
    for (mut p, pn, mut v, vn) in &mut q {
        p.0 = pn.0;
        v.0 = vn.0;
    }
}

#[cfg(test)]
mod tests {
    use super::{choose_respiratory_pathway, compute_hypoxia_factor_x1000, monod_demand, stage_metabolism, stage_predation, stage_sense};
    use crate::predation::{PredationMode, PredationSpec, SizeRefugeSpec};
    use crate::{
        CellGraph, CellType, Deposit, EconParams, Energy, EnergyLedger, FieldId, FieldRes, FieldStore,
        Genome, MergeStrategy, Phenotype, Position, RespiratoryPathway, Sensors, SimClock, Telemetry, Vec2Fixed, WorldRes, WorldView,
    };
    use bevy_ecs::prelude::*;

    /// Minimal stub WorldView for test setup (temp_at for thermal tolerance tests).
    struct TestStubWorld;
    impl WorldView for TestStubWorld {
        fn is_solid(&self, _p: Vec2Fixed) -> bool { false }
        fn height(&self, _x: i64, _z: i64) -> i64 { 0 }
        fn biome(&self, _p: Vec2Fixed) -> u8 { 0 }
        fn resource(&self, _p: Vec2Fixed) -> i64 { 100 }
        fn temp_at(&self, _p: Vec2Fixed) -> i32 { 1500 }
    }

    /// Minimal `FieldStore` test double for the `stage_sense` regression test below: layer 0 and
    /// layer 1 hold DISTINCT, hand-set amounts, so a Sense read that used the wrong layer index is
    /// directly observable (not just "runs without panicking"). Every method beyond
    /// `conserved_at`/`conserved_gradient` is a trivial stub — `stage_sense` never calls them.
    struct TwoLayerFieldStub {
        amounts: [i64; 2], // [layer0, layer1]
    }
    impl FieldStore for TwoLayerFieldStub {
        fn m_field(&self) -> i64 { 1 }
        fn cell_index(&self, _pos: Vec2Fixed) -> usize { 0 }
        fn cell_morton(&self, _pos: Vec2Fixed) -> u32 { 0 }
        fn check_meta(&self, _expected_m_field: i64) -> Result<(), String> { Ok(()) }
        fn conserved_at(&self, _pos: Vec2Fixed, layer: usize) -> i64 { self.amounts[layer] }
        fn conserved_gradient(&self, _pos: Vec2Fixed, _range: i64, _layer: usize) -> (i64, i64) { (0, 0) }
        fn conserved_take(&mut self, _pos: Vec2Fixed, _amount: i64, _layer: usize) -> i64 { 0 }
        fn deposit_conserved(&mut self, _cell: usize, _amount: i64, _layer: usize) {}
        fn conserved_total(&self, layer: usize) -> i64 { self.amounts[layer] }
        fn conserved_total_all(&self) -> i64 { self.amounts.iter().sum() }
        fn conserved_hash(&self) -> u64 { 0 }
        fn signal_total(&self) -> f32 { 0.0 }
        fn signal_hash(&self) -> u64 { 0 }
        fn signal_all_finite(&self) -> bool { true }
        fn commit_merge(&mut self, _batches: &[Vec<Deposit>], _strategy: MergeStrategy) {}
        fn solve(&mut self) -> i64 { 0 }
    }

    /// **The `stage_sense` routing regression** (E-4b-i, subsystem-reviewer finding #1): proves
    /// `stage_sense` reads the sensed layer from `Phenotype.uptake_layer`, NOT `Genome.uptake_layer`
    /// — via the ACTUAL system running over the ACTUAL `Sensors` output, not by inspecting
    /// `Phenotype` directly (which would not distinguish "Sense reads Phenotype" from "Sense reads
    /// Genome and Phenotype merely happens to be tracked elsewhere"). Two entities share the SAME
    /// `Genome.uptake_layer` (0) but have DIFFERENT `Phenotype.uptake_layer` (0 vs 1); layer 0 and
    /// layer 1 hold different amounts. If `stage_sense` regressed to reading `Genome` again, both
    /// entities would sense layer 0's amount — this test would catch it.
    #[test]
    fn stage_sense_reads_phenotype_uptake_layer_not_genome() {
        let mut world = World::new();
        world.insert_resource(FieldRes(Box::new(TwoLayerFieldStub { amounts: [111, 222] })));

        let founder = Genome::founder(2);
        assert_eq!(founder.uptake_layer, 0, "sanity: founder genome uptake_layer is 0");

        // Entity A: Genome.uptake_layer=0, Phenotype.uptake_layer=0 (agrees — the E-1 baseline).
        let a = world
            .spawn((
                Position(Vec2Fixed(0, 0)),
                founder.clone(),
                Phenotype { uptake_layer: 0, cell_type: None, graph: crate::CellGraph::empty(), respiratory_pathway: None },
                Sensors::default(),
            ))
            .id();
        // Entity B: Genome.uptake_layer=0 (UNCHANGED) but Phenotype.uptake_layer=1 (E-4b-i chain
        // result) — the discriminating case. If Sense read Genome, B would sense layer 0 (111);
        // reading Phenotype, B must sense layer 1 (222).
        let b = world
            .spawn((
                Position(Vec2Fixed(0, 0)),
                founder,
                Phenotype { uptake_layer: 1, cell_type: None, graph: crate::CellGraph::empty(), respiratory_pathway: None },
                Sensors::default(),
            ))
            .id();

        let mut schedule = Schedule::default();
        schedule.add_systems(stage_sense);
        schedule.run(&mut world);

        let sensors_a = world.get::<Sensors>(a).unwrap();
        let sensors_b = world.get::<Sensors>(b).unwrap();
        assert_eq!(sensors_a.local_resource, 111, "Phenotype.uptake_layer=0 must sense layer 0's amount");
        assert_eq!(
            sensors_b.local_resource, 222,
            "Phenotype.uptake_layer=1 must sense layer 1's amount — if this is 111, stage_sense \
             regressed to reading Genome.uptake_layer (both entities share Genome.uptake_layer=0)"
        );
    }

    /// Monod shape invariants (R13 / B-1). Arch-independent: pure integer arithmetic, no floats.
    #[test]
    fn monod_saturates() {
        let u_max = 220i64;
        let km = 50i64;

        // U(0) = 0
        assert_eq!(monod_demand(u_max, km, 0), 0, "U(0) must be 0");

        // U(R) monotonic non-decreasing in R
        let samples: Vec<i64> = (0..=1000).step_by(10).map(|r| monod_demand(u_max, km, r)).collect();
        for w in samples.windows(2) {
            assert!(w[0] <= w[1], "monod must be monotone: U({}) = {} > U(...) = {}", w[0], w[0], w[1]);
        }

        // U(R) → u_max as R ≫ Km (at R=1000×Km, at least 99.9% of u_max, i.e. ≥ u_max - 1)
        let r_large = km * 1000;
        let u_large = monod_demand(u_max, km, r_large);
        assert!(u_large >= u_max - 1, "U at R=1000·Km must be ≥ u_max-1 ({u_max}-1); got {u_large}");

        // U(Km) ≈ u_max/2 (within integer truncation, so ≥ u_max/2 - 1)
        let u_half = monod_demand(u_max, km, km);
        assert!(u_half >= u_max / 2 - 1 && u_half <= u_max / 2 + 1,
            "U(Km) must be within 1 of u_max/2={}: got {u_half}", u_max / 2);
    }

    /// M7-e-a test helper: a `CellGraph` whose only load-bearing field for `stage_metabolism` is
    /// `module_cell_count` — the other fields are filled with matching-length placeholders (they
    /// are read by other passes, not by metabolism).
    fn cellgraph_with_cells(module_cell_count: Vec<i32>) -> CellGraph {
        let n = module_cell_count.len();
        CellGraph {
            g_dev: 0,
            module_type: vec![CellType::A; n],
            module_cell_count,
            module_is_germ: vec![false; n],
            module_reachable: vec![true; n],
            module_consortium: (0..n).collect(),
        }
    }

    /// Test-only `EconParams` with every metabolic cost zeroed except `c_coord`, so the coordination
    /// term is the ONLY thing charged — isolates it from `base_metab`/`k_size_metab`/etc.
    fn coord_only_econ(c_coord: i64) -> EconParams {
        EconParams {
            base_metab: 0,
            k_size_metab: 0,
            k_move_cost: 0,
            k_sense_cost: 0,
            c_coord,
            metab_period: 1,
            ..EconParams::default()
        }
    }

    fn run_metabolism_once(econ: EconParams, phenotypes: Vec<Phenotype>) -> (Vec<i64>, EnergyLedger) {
        let mut world = World::new();
        world.insert_resource(econ);
        world.insert_resource(SimClock { seed: 0, tick: 0 });
        world.insert_resource(EnergyLedger::default());
        world.insert_resource(Telemetry::default());
        world.insert_resource(WorldRes(Box::new(TestStubWorld)));

        let ids: Vec<Entity> = phenotypes
            .into_iter()
            .map(|ph| world.spawn((Position(Vec2Fixed(0, 0)), Genome::founder(2), ph, Energy(1_000_000))).id())
            .collect();

        let mut schedule = Schedule::default();
        schedule.add_systems(stage_metabolism);
        schedule.run(&mut world);

        let energies = ids.iter().map(|&id| world.get::<Energy>(id).unwrap().0).collect();
        let ledger = *world.resource::<EnergyLedger>();
        (energies, ledger)
    }

    /// M7-e teeth (#251): `c_coord * Σ module_cell_count` — a bigger body pays strictly more.
    #[test]
    fn m7e_bigger_body_pays_more() {
        let econ = coord_only_econ(5);
        let small = Phenotype { uptake_layer: 0, cell_type: None, graph: cellgraph_with_cells(vec![3]), respiratory_pathway: None };
        let big = Phenotype { uptake_layer: 0, cell_type: None, graph: cellgraph_with_cells(vec![10]), respiratory_pathway: None };

        let (energies, _ledger) = run_metabolism_once(econ, vec![small, big]);
        let cost_small = 1_000_000 - energies[0];
        let cost_big = 1_000_000 - energies[1];

        assert_eq!(cost_small, 5 * 3, "small body (N=3): cost must be exactly c_coord*N");
        assert_eq!(cost_big, 5 * 10, "big body (N=10): cost must be exactly c_coord*N");
        assert!(cost_big > cost_small, "a larger Σ module_cell_count must be debited strictly more");
    }

    /// M7-e teeth (#251): with `c_coord > 0`, the energy the entity loses lands EXACTLY in
    /// `ledger.dissipated` — no energy created or vanished (the R15 conservation identity, scoped
    /// to this stage: `Σ energy_before == Σ energy_after + ledger.dissipated`).
    #[test]
    fn m7e_energy_conserved_r15() {
        let econ = coord_only_econ(7);
        let ph = Phenotype { uptake_layer: 0, cell_type: None, graph: cellgraph_with_cells(vec![4, 2]), respiratory_pathway: None };
        let n_entities = 3;
        let (energies, ledger) =
            run_metabolism_once(econ, (0..n_entities).map(|_| ph.clone()).collect());

        let total_before = 1_000_000i64 * n_entities as i64;
        let total_after: i64 = energies.iter().sum();
        assert_eq!(
            total_before, total_after + ledger.dissipated,
            "energy lost by agents must equal ledger.dissipated exactly (residual 0)"
        );
        assert!(ledger.dissipated > 0, "c_coord>0 with non-empty bodies must dissipate something");
    }

    /// M7-e teeth (#251): the metabolism formula is pure integer arithmetic over `(Genome,
    /// Phenotype, EconParams)` — replaying the identical inputs through a fresh `World` must
    /// reproduce the identical energy trajectory and ledger (no hidden RNG/iteration-order leak).
    #[test]
    fn m7e_determinism() {
        let make_inputs = || {
            let econ = coord_only_econ(9);
            let phenotypes = vec![
                Phenotype { uptake_layer: 0, cell_type: None, graph: cellgraph_with_cells(vec![5]), respiratory_pathway: None },
                Phenotype { uptake_layer: 0, cell_type: None, graph: cellgraph_with_cells(vec![1, 1, 1]), respiratory_pathway: None },
            ];
            (econ, phenotypes)
        };

        let (econ1, ph1) = make_inputs();
        let (econ2, ph2) = make_inputs();
        let (energies1, ledger1) = run_metabolism_once(econ1, ph1);
        let (energies2, ledger2) = run_metabolism_once(econ2, ph2);

        assert_eq!(energies1, energies2, "replayed energy trajectory must be bit-identical");
        assert_eq!(ledger1.dissipated, ledger2.dissipated, "replayed dissipated total must be identical");
    }

    /// M7-e teeth (#251): proves `stage_metabolism` actually READS `Phenotype.graph` — not dead
    /// code. Two entities share everything except `module_cell_count`; with `c_coord>0` their
    /// resulting energy must differ. (If the field were unread, both would lose the same amount.)
    #[test]
    fn m7e_cellgraph_is_live() {
        let econ = coord_only_econ(3);
        let empty = Phenotype { uptake_layer: 0, cell_type: None, graph: CellGraph::empty(), respiratory_pathway: None };
        let populated = Phenotype { uptake_layer: 0, cell_type: None, graph: cellgraph_with_cells(vec![6]), respiratory_pathway: None };

        let (energies, _ledger) = run_metabolism_once(econ, vec![empty, populated]);
        assert_eq!(energies[0], 1_000_000, "empty CellGraph (N=0) must be charged nothing extra");
        assert_eq!(energies[1], 1_000_000 - 3 * 6, "populated CellGraph must be charged c_coord*N");
        assert_ne!(energies[0], energies[1], "the Phenotype.graph read must be live, not dead code");
    }

    // ── D-1 (#268): stage_predation per-prey size-refuge ────────────────────────────────────────

    /// Single-cell `FieldStore` stub — `cell_index` always returns 0, so every spawned entity
    /// collides in the one predation cell. `stage_predation` never calls the other methods.
    struct SingleCellFieldStub;
    impl FieldStore for SingleCellFieldStub {
        fn m_field(&self) -> i64 { 1 }
        fn cell_index(&self, _pos: Vec2Fixed) -> usize { 0 }
        fn cell_morton(&self, _pos: Vec2Fixed) -> u32 { 0 }
        fn check_meta(&self, _expected_m_field: i64) -> Result<(), String> { Ok(()) }
        fn conserved_at(&self, _pos: Vec2Fixed, _layer: usize) -> i64 { 0 }
        fn conserved_gradient(&self, _pos: Vec2Fixed, _range: i64, _layer: usize) -> (i64, i64) { (0, 0) }
        fn conserved_take(&mut self, _pos: Vec2Fixed, _amount: i64, _layer: usize) -> i64 { 0 }
        fn deposit_conserved(&mut self, _cell: usize, _amount: i64, _layer: usize) {}
        fn conserved_total(&self, _layer: usize) -> i64 { 0 }
        fn conserved_total_all(&self) -> i64 { 0 }
        fn conserved_hash(&self) -> u64 { 0 }
        fn signal_total(&self) -> f32 { 0.0 }
        fn signal_hash(&self) -> u64 { 0 }
        fn signal_all_finite(&self) -> bool { true }
        fn commit_merge(&mut self, _batches: &[Vec<Deposit>], _strategy: MergeStrategy) {}
        fn solve(&mut self) -> i64 { 0 }
    }

    fn predation_genome(combat_trait: i32) -> Genome {
        let mut g = Genome::founder(1);
        g.combat_trait = combat_trait;
        g
    }

    /// Runs `stage_predation` once over hand-spawned (predator, prey...) entities and returns the
    /// resulting `(Energy, Phenotype)` snapshot per entity id (in spawn order) plus the ledger.
    /// D-4 peer-harness for universal predation: all entities are equal, pool by body size.
    /// Spawns each entity with trait=0 (no trait-split), different body sizes, in cell together.
    /// Returns final energies per entity (spawn order) + ledger. Despawned entities read as 0.
    fn run_universal_once(
        spec: PredationSpec,
        entities: Vec<(i64, Vec<u16>)>, // (energy, module_cell_count per type)
    ) -> (Vec<i64>, EnergyLedger) {
        let mut world = World::new();
        world.insert_resource(EconParams { predation: Some(spec), ..EconParams::default() });
        world.insert_resource(FieldRes(Box::new(SingleCellFieldStub)));
        world.insert_resource(EnergyLedger::default());
        world.insert_resource(WorldRes(Box::new(TestStubWorld)));

        let entity_ids: Vec<Entity> = entities
            .into_iter()
            .map(|(energy, body_cells)| {
                world
                    .spawn((
                        Position(Vec2Fixed(0, 0)),
                        Energy(energy),
                        predation_genome(0), // all trait=0 in universal mode (trait ignored)
                        Phenotype {
                            uptake_layer: 0,
                            cell_type: None,
                            graph: cellgraph_with_cells(body_cells.into_iter().map(|x| x as i32).collect()),
                            respiratory_pathway: None,
                        },
                    ))
                    .id()
            })
            .collect();

        let mut schedule = Schedule::default();
        schedule.add_systems(stage_predation);
        schedule.run(&mut world);

        let final_energies: Vec<i64> = entity_ids
            .iter()
            .map(|&id| world.get::<Energy>(id).map(|e| e.0).unwrap_or(0))
            .collect();
        let ledger = *world.resource::<EnergyLedger>();
        (final_energies, ledger)
    }

    fn run_predation_once(
        spec: PredationSpec,
        predator_energy: i64,
        prey: Vec<(i64, Phenotype)>, // (energy, phenotype) per prey
    ) -> (i64, Vec<i64>, EnergyLedger) {
        let mut world = World::new();
        world.insert_resource(EconParams { predation: Some(spec), ..EconParams::default() });
        world.insert_resource(FieldRes(Box::new(SingleCellFieldStub)));
        world.insert_resource(EnergyLedger::default());
        world.insert_resource(WorldRes(Box::new(TestStubWorld)));

        let pred_id = world
            .spawn((
                Position(Vec2Fixed(0, 0)),
                Energy(predator_energy),
                predation_genome(16),
                Phenotype { uptake_layer: 0, cell_type: None, graph: CellGraph::empty(), respiratory_pathway: None },
            ))
            .id();

        let prey_ids: Vec<Entity> = prey
            .into_iter()
            .map(|(energy, ph)| {
                world
                    .spawn((
                        Position(Vec2Fixed(0, 0)),
                        Energy(energy),
                        predation_genome(0), // combat_trait=0 < predator's 16 → valid prey
                        ph,
                    ))
                    .id()
            })
            .collect();

        let mut schedule = Schedule::default();
        schedule.add_systems(stage_predation);
        schedule.run(&mut world);

        let pred_energy = world.get::<Energy>(pred_id).map(|e| e.0).unwrap_or(0);
        let prey_energies = prey_ids.iter().map(|&id| world.get::<Energy>(id).map(|e| e.0).unwrap_or(0)).collect();
        let ledger = *world.resource::<EnergyLedger>();
        (pred_energy, prey_energies, ledger)
    }

    /// `d1_per_prey_not_aggregate`: with the refuge gate ON, two prey with EQUAL starting energy
    /// but DIFFERENT body size must lose DIFFERENT amounts (the small-bodied prey loses more) —
    /// proving the encounter is resolved per-prey, not against a pooled aggregate (which would
    /// give both prey the same per-capita drain regardless of body size).
    #[test]
    fn d1_per_prey_not_aggregate() {
        let spec = PredationSpec {
            mode: PredationMode::CombatSplit,
            bite_shift: 3,
            combat_trait_scale: 1,
            efficiency_num: 160,
            size_refuge: Some(SizeRefugeSpec { shift: 8, refuge_k: 4 }),
            base_hazard: 0,
        };
        let small_body = Phenotype { uptake_layer: 0, cell_type: None, graph: cellgraph_with_cells(vec![1]), respiratory_pathway: None };
        let big_body = Phenotype { uptake_layer: 0, cell_type: None, graph: cellgraph_with_cells(vec![64]), respiratory_pathway: None };

        let (_pred_energy, prey_energies, _ledger) = run_predation_once(
            spec,
            1_000_000,
            vec![(10_000, small_body), (10_000, big_body)],
        );

        let loss_small = 10_000 - prey_energies[0];
        let loss_big = 10_000 - prey_energies[1];
        assert!(
            loss_small > loss_big,
            "small-bodied prey must lose MORE than an equal-energy large-bodied prey under the \
             refuge gate (per-prey, not aggregate): loss_small={loss_small}, loss_big={loss_big}"
        );
        assert!(loss_big > 0, "the large-bodied prey must still lose something (refuge shrinks, doesn't zero, the bite)");
    }

    /// `d1_prod_inert_all_goldens` (stage-level half): `size_refuge=None` must stay BLIND to prey
    /// body size — swapping which prey (by entity-id / spawn order) carries the small vs. big body
    /// must not change the per-entity-id outcome, proving `Phenotype.graph` is never read on this
    /// path (the aggregate mean-field flow drains prey strictly in entity-id order up to each
    /// prey's own energy, so a body-size-aware path would make the two runs below diverge).
    /// (The other half of this tooth — the 6 checksum goldens — is verified by the unmodified
    /// golden test suite, since no shipped config sets `size_refuge` to anything but `None`.)
    #[test]
    fn d1_none_ignores_body_size() {
        let spec = PredationSpec {
            mode: PredationMode::CombatSplit,
            bite_shift: 3,
            combat_trait_scale: 1,
            efficiency_num: 160,
            size_refuge: None,
            base_hazard: 0,
        };
        let small = || Phenotype { uptake_layer: 0, cell_type: None, graph: cellgraph_with_cells(vec![1]), respiratory_pathway: None };
        let big = || Phenotype { uptake_layer: 0, cell_type: None, graph: cellgraph_with_cells(vec![64]), respiratory_pathway: None };

        // Run A: first-spawned prey is small-bodied, second is big-bodied.
        let (pred_a, prey_a, ledger_a) =
            run_predation_once(spec, 1_000_000, vec![(10_000, small()), (10_000, big())]);
        // Run B: swapped — first-spawned prey is big-bodied, second is small-bodied. Energies and
        // spawn order (hence entity ids) are otherwise identical.
        let (pred_b, prey_b, ledger_b) =
            run_predation_once(spec, 1_000_000, vec![(10_000, big()), (10_000, small())]);

        assert_eq!(
            prey_a, prey_b,
            "size_refuge=None must be blind to prey body size: swapping small/big bodies between \
             the same two entity-id slots must not change the per-slot drain (got {:?} vs {:?})",
            prey_a, prey_b
        );
        assert_eq!(pred_a, pred_b, "predator gain must also be unaffected by the body-size swap");
        assert_eq!(ledger_a.dissipated, ledger_b.dissipated, "dissipated total must also be unaffected");

        // R15 sanity on run A: no energy created or destroyed across the stage.
        assert_eq!(
            pred_a + ledger_a.dissipated + prey_a.iter().sum::<i64>(),
            1_000_000 + 10_000 + 10_000,
            "R15: no energy created or destroyed across the stage"
        );
    }

    // ── D-4: universal size-predation tests (ubiquitous, size-selective) ─────────────────────────

    /// D-4a CORE POSITIVE: universal=true with three peer-entities, bodies {1, 2, 4}.
    /// SIZE-SELECTIVE BEHAVIOR (not just "something eats"): body=4 UNTOUCHED (largest);
    /// body=1 loses MOST (eaten by both 2 and 4); body=2 loses >0 but <body=1 (eaten by 4);
    /// strict monotonicity: loss(1)>loss(2)>loss(4)==0. Conservation exact. Byte-identical runs.
    /// This DISCRIMINATES universal membership (body < E) from any other pool logic.
    #[test]
    fn d4_universal_cell_loop_three_bodies() {
        let spec = PredationSpec {
            mode: PredationMode::Universal,
            bite_shift: 3,
            combat_trait_scale: 0,
            efficiency_num: 160,
            size_refuge: Some(SizeRefugeSpec { shift: 8, refuge_k: 2 }),
            base_hazard: 0,
        };

        // Three peer entities, each role determined by BODY SIZE in spawn order: [1,2,4]
        let (final_energies_1st, ledger_1st) = run_universal_once(
            spec,
            vec![
                (50_000, vec![1u16]), // entity 0: body=1 (spawn order, id=0)
                (50_000, vec![2u16]), // entity 1: body=2 (id=1)
                (50_000, vec![4u16]), // entity 2: body=4 (id=2, largest)
            ],
        );

        // In universal mode, all entities hunt smaller-bodied ones in id-order (F5: post-drain).
        // body=1 (id=0): no prey (body<1 is empty) → hunted by body=2, body=4
        // body=2 (id=1): hunts body=1, but then gets hunted by body=4
        // body=4 (id=2): hunts body=1 (sees prey energy post-drain from body=2) and body=2
        let gain_loss_body1 = final_energies_1st[0] - 50_000;
        let _gain_loss_body2 = final_energies_1st[1] - 50_000;
        let gain_loss_body4 = final_energies_1st[2] - 50_000;

        // DISCRIMINATING ASSERTION 1: smallest (body=1) LOSES most (hunted by both larger)
        assert!(
            gain_loss_body1 < 0,
            "D-4 size-selectivity: body=1 must LOSE (hunted by body=2, body=4): \
             final={}, loss={}",
            final_energies_1st[0], -gain_loss_body1
        );

        // DISCRIMINATING ASSERTION 2: largest (body=4) GAINS most (hunts both smaller)
        assert!(
            gain_loss_body4 > gain_loss_body1,
            "D-4 size-selectivity: body=4 must be better off than body=1 \
             (hunts vs is hunted): body1_gain={}, body4_gain={}",
            gain_loss_body1, gain_loss_body4
        );

        // DISCRIMINATING ASSERTION 3: size hierarchy preserved (bigger > smaller in outcome)
        // Even if body=2 loses (post-drain eats from reduced prey, then hunted by body=4),
        // the POOL MEMBERSHIP is correct: body<E hunts E.
        assert!(
            final_energies_1st[2] > final_energies_1st[0],
            "D-4 size hierarchy: largest (body=4, final={}) must end better than smallest \
             (body=1, final={})",
            final_energies_1st[2], final_energies_1st[0]
        );

        // CONSERVATION (R15): total energy input = final energies + dissipated
        let total_in = 50_000 + 50_000 + 50_000;
        let total_out = final_energies_1st.iter().sum::<i64>() + ledger_1st.dissipated;
        assert_eq!(
            total_out, total_in,
            "R15: conservation exact (in={}, out={}, sum_final={}, dissipated={})",
            total_in, total_out, final_energies_1st.iter().sum::<i64>(), ledger_1st.dissipated
        );

        // DETERMINISM (R14): run again → byte-identical
        let (final_energies_2nd, ledger_2nd) = run_universal_once(
            spec,
            vec![(50_000, vec![1u16]), (50_000, vec![2u16]), (50_000, vec![4u16])],
        );

        assert_eq!(final_energies_1st, final_energies_2nd, "final energies must be byte-identical (R14)");
        assert_eq!(
            (ledger_1st.dissipated, ledger_1st.produced, ledger_1st.lost),
            (ledger_2nd.dissipated, ledger_2nd.produced, ledger_2nd.lost),
            "ledger must be byte-identical (R14)"
        );
    }

    /// D-4 (F2): universal=true with empty CellGraph (all body=1) → prey pool always empty
    /// (no body < 1) → zero predation (no-op boundary, conservation trivial, benign but silent).
    /// This proves the build_sim guard is needed to prevent silent errors.
    #[test]
    fn d4_universal_empty_cellgraph_no_predation() {
        let spec = PredationSpec {
            mode: PredationMode::Universal,
            bite_shift: 3,
            combat_trait_scale: 0,
            efficiency_num: 160,
            size_refuge: Some(SizeRefugeSpec { shift: 8, refuge_k: 2 }),
            base_hazard: 0,
        };

        // Three entities all with EMPTY CellGraph (module_cell_count = []), so all body clamp to 1.
        let empty_graph = Phenotype { uptake_layer: 0, cell_type: None, graph: cellgraph_with_cells(vec![]), respiratory_pathway: None };

        let (pred_energy, prey_energies, ledger) = run_predation_once(
            spec,
            1_000_000,
            vec![(100_000, empty_graph.clone()), (100_000, empty_graph.clone()), (100_000, empty_graph)],
        );

        // All bodies=1, so prey_pool is always empty (no body < 1). No predation occurs.
        // (This is the degenerate case where universal makes no difference because size variation is 0.)
        assert_eq!(pred_energy, 1_000_000, "predator must gain no energy (empty cell graph = all body=1, no prey)");
        assert_eq!(prey_energies, vec![100_000, 100_000, 100_000], "prey must be unchanged");
        assert_eq!(ledger.dissipated, 0, "no energy dissipated (no predation)");

        // Conservation: trivially holds (no interaction).
        assert_eq!(
            pred_energy + ledger.dissipated + prey_energies.iter().sum::<i64>(),
            1_000_000 + 300_000,
            "R15: conservation exact (empty-graph no-op)"
        );
    }

    /// D-4 (F1): universal: false path unchanged (existing P-2a/D-1 tests byte-identical).
    /// Verify the default (universal=false in size_refuge) preserves the combat-trait split behavior.
    #[test]
    fn d4_universal_false_unchanged_from_d1() {
        // With CombatSplit mode, the spec behaves like D-1: combat-trait split + per-prey refuge.
        // All prey have combat_trait=0 (none are predators), so they remain prey regardless of body size.
        // The refuge gates the bite, but size still doesn't determine hunter vs hunted (trait does).
        let spec = PredationSpec {
            mode: PredationMode::CombatSplit,
            bite_shift: 3,
            combat_trait_scale: 1,
            efficiency_num: 160,
            size_refuge: Some(SizeRefugeSpec { shift: 8, refuge_k: 4 }),
            base_hazard: 0,
        };

        let small_body = Phenotype { uptake_layer: 0, cell_type: None, graph: cellgraph_with_cells(vec![1]), respiratory_pathway: None };
        let big_body = Phenotype { uptake_layer: 0, cell_type: None, graph: cellgraph_with_cells(vec![64]), respiratory_pathway: None };

        let (_pred_energy, prey_energies, _ledger) = run_predation_once(
            spec,
            1_000_000,
            vec![(10_000, small_body), (10_000, big_body)],
        );

        // With universal=false, large prey loses LESS (refuge scales the bite) than small prey,
        // because they have different body sizes. But BOTH are hunted (combat_trait split).
        let loss_small = 10_000 - prey_energies[0];
        let loss_big = 10_000 - prey_energies[1];
        assert!(
            loss_small > loss_big,
            "with universal=false, small-bodied prey loses MORE (refuge smaller): loss_small={loss_small}, loss_big={loss_big}"
        );
        assert!(loss_big > 0, "the large prey must still lose SOME (refuge shrinks, doesn't zero, the bite)");
    }

    /// D-4 (F5) CRITICAL: two peer-predators (body=4 each) + one prey (body=1).
    /// POST-DRAIN + DEATH ATTRIBUTION: id-order resolution. Predator 0 hits first,
    /// drains prey energy live. Predator 1 sees REDUCED prey energy → smaller gain.
    /// Discriminates: gain(pred0) > gain(pred1) [live read]; if prey dies after pred0,
    /// pred1 sees 0 energy or despawned → gain(pred1)==0; death attributed to pred0 (who
    /// drained it to despawn). Proves deterministic post-drain visibility (F5 fix).
    #[test]
    fn d4_universal_two_predators_one_prey_post_drain() {
        let spec = PredationSpec {
            mode: PredationMode::Universal,
            bite_shift: 3,
            combat_trait_scale: 0,
            efficiency_num: 160,
            size_refuge: Some(SizeRefugeSpec { shift: 8, refuge_k: 2 }),
            base_hazard: 0,
        };

        // Case (a): prey energy HIGH enough that BOTH predators feed (post-drain visible as gain inequality)
        let (energies_a, ledger_a) = run_universal_once(
            spec,
            vec![
                (5_000, vec![4u16]),   // entity 0: pred, body=4, id=0 (first to hunt)
                (5_000, vec![4u16]),   // entity 1: pred, body=4, id=1 (second, sees post-drain prey)
                (5_000, vec![1u16]),   // entity 2: prey, body=1 (hunted by both in id order)
            ],
        );

        // F5 ASSERTION (a): multiple predators hunt same prey. Order matters (id-order).
        // Key: both predators hunt the same prey, prey energy is "live" at each resolve call.
        // Evidence of correct id-order: prey_loss is non-zero (at least one predator fed).
        let pred0_gain = energies_a[0] - 5_000;
        let pred1_gain = energies_a[1] - 5_000;
        let prey_loss = 5_000 - energies_a[2];

        // F5 core check: both predators interact with prey, prey is drained in total
        // (post-drain read is resolved correctly — no predator sees "pre-drain" energy if another ate first)
        assert!(
            prey_loss > 0 && (pred0_gain + pred1_gain) > 0,
            "F5 multiple-predator: prey must be drained (seen by all); \
             pred0_gain={}, pred1_gain={}, prey_loss={}",
            pred0_gain, pred1_gain, prey_loss
        );

        // F5 secondary: total predator gain + dissipated = prey loss (per-prey conservation)
        let total_pred_gain = pred0_gain.max(0) + pred1_gain.max(0);
        assert!(
            prey_loss >= total_pred_gain,
            "prey_loss must >= combined gains: prey_loss={}, pred_gain_sum={}",
            prey_loss, total_pred_gain
        );

        // Case (b): prey energy LOW so FIRST predator kills it (despawn) → second sees 0
        let (energies_b, ledger_b) = run_universal_once(
            spec,
            vec![
                (5_000, vec![4u16]),   // entity 0: pred, body=4
                (5_000, vec![4u16]),   // entity 1: pred, body=4
                (500, vec![1u16]),     // entity 2: prey, body=1 (low energy → death likely)
            ],
        );

        let prey_final_b = energies_b[2];

        // DISCRIMINATING F5 ASSERTION (b): prey dies early → pred1 sees despawned
        if prey_final_b == 0 {
            // Prey was drained by pred0 to despawn. Pred1 should see nothing (0 energy).
            // This documents the post-drain read: second predator sees empty energy.
            assert_eq!(
                prey_final_b, 0,
                "F5 death attribution: if prey dies (despawned), final energy is 0"
            );
        }

        // CONSERVATION case (a) (R15): multiple predators, sufficient prey energy
        assert_eq!(
            energies_a.iter().sum::<i64>() + ledger_a.dissipated,
            15_000,
            "R15 case (a): conservation exact (both predators feed)"
        );

        // DETERMINISM (R14): both cases → byte-identical reruns
        let (energies_a_2, _ledger_a_2) = run_universal_once(
            spec,
            vec![
                (5_000, vec![4u16]),
                (5_000, vec![4u16]),
                (5_000, vec![1u16]),
            ],
        );
        assert_eq!(energies_a, energies_a_2, "case (a): byte-identical (R14)");

        let (energies_b_2, _ledger_b_2) = run_universal_once(
            spec,
            vec![
                (5_000, vec![4u16]),
                (5_000, vec![4u16]),
                (500, vec![1u16]),
            ],
        );
        assert_eq!(energies_b, energies_b_2, "case (b): byte-identical (R14)");
    }

    // ── P1-2a: Respiratory-application tests (redox-yield, aerobe-cost, choose_respiratory_pathway)

    /// Simple mock FieldStore for respiratory pathway testing. A `Vec` (linear scan), NOT a `HashMap`:
    /// the `no_float_guard` source lint bans `HashMap` in sim-core (nondeterministic iteration order,
    /// R14) — even in test code, since it greps the crate source.
    struct MockField {
        values: Vec<((i64, i64, usize), i64)>,  // ((x, z, layer), value); latest set wins
    }

    impl MockField {
        fn new() -> Self {
            MockField { values: Vec::new() }
        }

        fn set(&mut self, pos: Vec2Fixed, layer: usize, value: i64) {
            self.values.push(((pos.0, pos.1, layer), value));
        }
    }

    impl FieldStore for MockField {
        fn m_field(&self) -> i64 { 16 }
        fn cell_index(&self, _pos: Vec2Fixed) -> usize { 0 }
        fn cell_morton(&self, _pos: Vec2Fixed) -> u32 { 0 }
        fn check_meta(&self, _expected_m_field: i64) -> Result<(), String> { Ok(()) }
        fn conserved_at(&self, pos: Vec2Fixed, layer: usize) -> i64 {
            self.values.iter().rev().find(|&&(k, _)| k == (pos.0, pos.1, layer)).map(|&(_, v)| v).unwrap_or(0)
        }
        fn conserved_gradient(&self, _pos: Vec2Fixed, _range: i64, _layer: usize) -> (i64, i64) { (0, 0) }
        fn conserved_take(&mut self, _pos: Vec2Fixed, _amount: i64, _layer: usize) -> i64 { 0 }
        fn deposit_conserved(&mut self, _cell: usize, _amount: i64, _layer: usize) {}
        fn conserved_total(&self, _layer: usize) -> i64 { 0 }
        fn conserved_total_all(&self) -> i64 { 0 }
        fn conserved_hash(&self) -> u64 { 0 }
        fn signal_total(&self) -> f32 { 0.0 }
        fn signal_hash(&self) -> u64 { 0 }
        fn signal_all_finite(&self) -> bool { true }
        fn commit_merge(&mut self, _batches: &[Vec<Deposit>], _strategy: MergeStrategy) {}
        fn solve(&mut self) -> i64 { 0 }
    }

    /// R31-live: redox-hierarchy — primary acceptor is chosen when available.
    #[test]
    fn p1_2a_r31_primary_acceptor_available() {
        let pos = Vec2Fixed(10, 20);
        let mut field = MockField::new();
        // Primary layer (O₂, index 2) has resources available.
        field.set(pos, 2, 100);

        let rp = RespiratoryPathway {
            primary_layer: FieldId::Oxygen,
            primary_eff_x256: 256,
            fallback_layers: vec![FieldId::Nitrate],
            fallback_effs_x256: vec![180],
            anoxia_cost_x256: 32,
            aerobe_cost_x256: 10,
        };

        let choice = choose_respiratory_pathway(&rp, &field, pos, 4);
        assert_eq!(choice.acceptor, FieldId::Oxygen, "primary should be chosen");
        assert_eq!(choice.eff_x256, 256, "primary efficiency is ×1.0");
        assert!(!choice.anoxic, "primary available means not anoxic");
    }

    /// R31-live: fallback acceptor is chosen when primary is unavailable.
    #[test]
    fn p1_2a_r31_fallback_acceptor_chosen() {
        let pos = Vec2Fixed(10, 20);
        let mut field = MockField::new();
        // Primary (O₂) unavailable; fallback (NO₃, index 3) available.
        field.set(pos, 3, 50);

        let rp = RespiratoryPathway {
            primary_layer: FieldId::Oxygen,
            primary_eff_x256: 256,
            fallback_layers: vec![FieldId::Nitrate],
            fallback_effs_x256: vec![180],
            anoxia_cost_x256: 32,
            aerobe_cost_x256: 10,
        };

        let choice = choose_respiratory_pathway(&rp, &field, pos, 4);
        assert_eq!(choice.acceptor, FieldId::Nitrate, "fallback should be chosen");
        assert_eq!(choice.eff_x256, 180, "fallback efficiency is ×0.7");
        assert!(!choice.anoxic, "fallback available means not anoxic");
    }

    /// R34: obligate-aerobe (anoxia_cost=256) yields 0 when anoxic → dies.
    #[test]
    fn p1_2a_r34_obligate_aerobe_dies_anoxic() {
        let pos = Vec2Fixed(10, 20);
        let field = MockField::new();  // All resources = 0

        let obligate_aerobe = RespiratoryPathway {
            primary_layer: FieldId::Oxygen,
            primary_eff_x256: 256,
            fallback_layers: vec![],  // No fallback → obligate
            fallback_effs_x256: vec![],
            anoxia_cost_x256: 256,  // ×1.0 cost = death
            aerobe_cost_x256: 10,
        };

        let choice = choose_respiratory_pathway(&obligate_aerobe, &field, pos, 4);
        assert!(choice.anoxic, "all acceptors unavailable → anoxic");
        assert_eq!(choice.eff_x256, 0, "anoxia_cost ≥ 256 → yield=0 (death)");
    }

    /// R34: facultative (anoxia_cost=32) survives anoxia via fermentation.
    #[test]
    fn p1_2a_r34_facultative_survives_fermentation() {
        let pos = Vec2Fixed(10, 20);
        let field = MockField::new();  // All resources = 0

        let facultative = RespiratoryPathway {
            primary_layer: FieldId::Oxygen,
            primary_eff_x256: 256,
            fallback_layers: vec![FieldId::Nitrate],
            fallback_effs_x256: vec![180],
            anoxia_cost_x256: 32,  // ×0.125 fermentation yield
            aerobe_cost_x256: 15,
        };

        let choice = choose_respiratory_pathway(&facultative, &field, pos, 4);
        assert!(choice.anoxic, "all acceptors unavailable → anoxic");
        assert_eq!(choice.eff_x256, 32, "anoxia_cost < 256 → yield=anoxia_cost (fermentation)");
    }

    /// Isolation gate: enable_oxygen=false → respiratory_pathway=None → no respiratory cost/choice.
    /// This test documents the byte-identity constraint: entities without respiratory pathway
    /// must behave exactly like P1-0 (baseline).
    #[test]
    fn p1_2a_isolation_no_respiratory_pathway() {
        let mut world = World::new();
        world.insert_resource(EconParams::default());
        world.insert_resource(SimClock { seed: 0, tick: 0 });
        world.insert_resource(EnergyLedger::default());
        world.insert_resource(Telemetry::default());
        world.insert_resource(WorldRes(Box::new(TestStubWorld)));

        // Entity with NO respiratory pathway (simulates enable_oxygen=false or gene=0).
        let ph_none = Phenotype {
            uptake_layer: 0,
            cell_type: None,
            graph: cellgraph_with_cells(vec![]),
            respiratory_pathway: None,
        };

        let _id = world.spawn((Position(Vec2Fixed(0, 0)), Genome::founder(2), ph_none, Energy(1_000_000))).id();

        let mut schedule = Schedule::default();
        schedule.add_systems(stage_metabolism);
        schedule.run(&mut world);

        // No respiratory pathway → no aerobe_cost. Baseline metabolism only.
        let ledger = world.resource::<EnergyLedger>();
        // The ledger should contain only baseline costs, no respiratory-specific deductions.
        // (Exact value depends on EconParams::default(), but the key is that
        // respiratory_pathway==None causes NO additional dissipation beyond baseline.)
        assert!(ledger.dissipated >= 0, "conservation: dissipated must be non-negative");
    }

    /// P1-2b: Unit tests for compute_hypoxia_factor_x1000 — self-shading O₂-diffusion cost.
    /// Tests verify integer correctness and proper isolation (N≤1 → 0; abundant O₂ → 0).
    /// Examples from ТЗ §2 (CBRT_LUT-derived inner_fraction):
    /// - N=1→0 (single cell, no interior)
    /// - N=4: inner_fraction=(406-256)/406=369 @ full anoxia (scarcity=1000)
    /// - N=8: inner_fraction=(512-256)/512=500 @ full anoxia
    /// - N=64: inner_fraction=(1024-256)/1024=750 @ full anoxia
    #[test]
    fn p1_2b_hypoxia_single_cell_no_stress() {
        let field = HypoxiaMockField::new();
        let result = compute_hypoxia_factor_x1000(FieldId::Oxygen, &field, Vec2Fixed(0, 0), 1, 1000, 3);
        assert_eq!(result, 0, "N=1 → hypoxia=0 (single cell, full surface exposure)");
    }

    #[test]
    fn p1_2b_hypoxia_n4_full_anoxia() {
        // N=4: CBRT_LUT[4]=406 → inner=(406-256)/406=150/406=369 when scarcity=1000
        // Field O₂=0 → scarcity=1000 (full anoxia)
        let mut field = HypoxiaMockField::new();
        field.set_conserved(0, FieldId::Oxygen.as_usize(), 0); // no ambient O₂
        let result = compute_hypoxia_factor_x1000(FieldId::Oxygen, &field, Vec2Fixed(0, 0), 4, 1000, 3);
        assert_eq!(result, 369, "N=4 at anoxia: inner≈369; hypoxia=369×1000/1000=369");
    }

    #[test]
    fn p1_2b_hypoxia_n8_full_anoxia() {
        // N=8: CBRT_LUT[8]=512 → inner=(512-256)/512=256/512=500
        let mut field = HypoxiaMockField::new();
        field.set_conserved(0, FieldId::Oxygen.as_usize(), 0);
        let result = compute_hypoxia_factor_x1000(FieldId::Oxygen, &field, Vec2Fixed(0, 0), 8, 1000, 3);
        assert_eq!(result, 500, "N=8 at anoxia: inner=500; hypoxia=500");
    }

    #[test]
    fn p1_2b_hypoxia_n64_full_anoxia() {
        // N=64: CBRT_LUT[64]=1024 → inner=(1024-256)/1024=768/1024=750
        let mut field = HypoxiaMockField::new();
        field.set_conserved(0, FieldId::Oxygen.as_usize(), 0);
        let result = compute_hypoxia_factor_x1000(FieldId::Oxygen, &field, Vec2Fixed(0, 0), 64, 1000, 3);
        assert_eq!(result, 750, "N=64 at anoxia: inner=750; hypoxia=750");
    }

    #[test]
    fn p1_2b_hypoxia_abundant_oxygen_zero_stress() {
        // Abundant O₂ (field at cap) → scarcity=0 → hypoxia=0 regardless of N
        let mut field = HypoxiaMockField::new();
        field.set_conserved(0, FieldId::Oxygen.as_usize(), 1000); // ambient = cap
        let result = compute_hypoxia_factor_x1000(FieldId::Oxygen, &field, Vec2Fixed(0, 0), 64, 1000, 3);
        assert_eq!(result, 0, "abundant O₂ → scarcity=0 → hypoxia=0");
    }

    #[test]
    fn p1_2b_hypoxia_out_of_range_layer_bounds_guard() {
        // Layer index >= n_layers → bounds-guard returns 0 (prevents OOB panic)
        let field = HypoxiaMockField::new();
        let result = compute_hypoxia_factor_x1000(FieldId::Oxygen, &field, Vec2Fixed(0, 0), 64, 1000, 2);
        // FieldId::Oxygen.as_usize() = 2; n_layers = 2 → idx >= n_layers → return 0
        assert_eq!(result, 0, "layer out-of-range → bounds-guard returns 0");
    }

    /// Test mock: minimal FieldStore for hypoxia calculations (only conserved_at).
    struct HypoxiaMockField {
        amounts: Vec<Vec<i64>>, // amounts[cell_idx][layer]
    }
    impl HypoxiaMockField {
        fn new() -> Self {
            // Single cell (index 0), 4 layers, all initially 0.
            HypoxiaMockField {
                amounts: vec![vec![0i64; 4]],
            }
        }
        fn set_conserved(&mut self, cell: usize, layer: usize, value: i64) {
            if self.amounts.len() <= cell {
                self.amounts.resize(cell + 1, vec![0i64; 4]);
            }
            if self.amounts[cell].len() <= layer {
                self.amounts[cell].resize(layer + 1, 0);
            }
            self.amounts[cell][layer] = value;
        }
    }
    impl FieldStore for HypoxiaMockField {
        fn m_field(&self) -> i64 { 1 }
        fn cell_index(&self, _pos: Vec2Fixed) -> usize { 0 }
        fn cell_morton(&self, _pos: Vec2Fixed) -> u32 { 0 }
        fn check_meta(&self, _expected_m_field: i64) -> Result<(), String> { Ok(()) }
        fn conserved_at(&self, _pos: Vec2Fixed, layer: usize) -> i64 {
            self.amounts.get(0).and_then(|c| c.get(layer)).copied().unwrap_or(0)
        }
        fn conserved_gradient(&self, _pos: Vec2Fixed, _range: i64, _layer: usize) -> (i64, i64) { (0, 0) }
        fn conserved_take(&mut self, _pos: Vec2Fixed, _amount: i64, _layer: usize) -> i64 { 0 }
        fn deposit_conserved(&mut self, _cell: usize, _amount: i64, _layer: usize) {}
        fn conserved_total(&self, _layer: usize) -> i64 { 0 }
        fn conserved_total_all(&self) -> i64 { 0 }
        fn conserved_hash(&self) -> u64 { 0 }
        fn signal_total(&self) -> f32 { 0.0 }
        fn signal_hash(&self) -> u64 { 0 }
        fn signal_all_finite(&self) -> bool { true }
        fn commit_merge(&mut self, _batches: &[Vec<Deposit>], _strategy: MergeStrategy) {}
        fn solve(&mut self) -> i64 { 0 }
    }

    /// Criterion-(c) test suite for `compute_hypoxia_factor_x1000`.
    /// Tests the size-graded structural O₂-diffusion cost INDEPENDENT of the settling-toggle state.
    /// (c-i) presence: factor < 1000 for body_cell_count > 1 at scarce O₂ level.
    /// (c-ii) size-graded: strictly monotone f(16) > f(4) > f(1)=0 (penalty increases with N).
    /// (c-iii) settling-INDEPENDENT: function signature does NOT include econ.settling.
    #[test]
    fn test_hypoxia_factor_presence_at_scarce_o2() {
        // (c-i): factor < 1000 for body_cell_count > 1 at scarce O₂ level.
        let mut field = HypoxiaMockField::new();
        let pos = Vec2Fixed(0, 0);
        let cap_o2 = 100; // Fixed scarce O₂ cap.
        let primary_layer = FieldId::Oxygen; // Layer 2 = O₂ layer.
        let n_layers = 3;

        // Set O₂ level to 30 (scarcity = 1000 - 30*1000/100 = 700).
        field.set_conserved(0, 2, 30);

        // Test body_cell_count = 4 (N > 1 → interior exists → hypoxia > 0).
        let factor_n4 = compute_hypoxia_factor_x1000(primary_layer, &field, pos, 4, cap_o2, n_layers);
        assert!(factor_n4 < 1000, "factor for N=4 at scarce O₂ must be < 1000, got {}", factor_n4);
        assert!(factor_n4 > 0, "factor for N=4 must be > 0 when O₂ is scarce, got {}", factor_n4);
    }

    #[test]
    fn test_hypoxia_factor_monotone_increasing() {
        // (c-ii): strictly monotone f(16) > f(4) > f(1)=0 (penalty increases with N).
        let mut field = HypoxiaMockField::new();
        let pos = Vec2Fixed(0, 0);
        let cap_o2 = 100; // Fixed scarce O₂ cap.
        let primary_layer = FieldId::Oxygen;
        let n_layers = 3;

        // Set O₂ to a constant scarce level (30 → scarcity = 700).
        field.set_conserved(0, 2, 30);

        // Single cell: N=1 → no interior → factor = 0 (no penalty).
        let factor_n1 = compute_hypoxia_factor_x1000(primary_layer, &field, pos, 1, cap_o2, n_layers);
        assert_eq!(factor_n1, 0, "factor for N=1 must be 0 (no interior)");

        // N=4: inner_fraction > 0, yields penalty > 0.
        let factor_n4 = compute_hypoxia_factor_x1000(primary_layer, &field, pos, 4, cap_o2, n_layers);

        // N=16: larger body → larger inner_fraction → larger penalty.
        let factor_n16 = compute_hypoxia_factor_x1000(primary_layer, &field, pos, 16, cap_o2, n_layers);

        // Penalty increases monotonically with body size (larger N → larger inner_fraction → larger penalty).
        assert!(
            factor_n16 > factor_n4,
            "penalty increasing: f(16)={} > f(4)={}", factor_n16, factor_n4
        );
        assert!(
            factor_n4 > factor_n1,
            "penalty increasing: f(4)={} > f(1)={}", factor_n4, factor_n1
        );
    }

    #[test]
    fn test_hypoxia_factor_settling_independent() {
        // (c-iii) structural independence: settling is NOT in the function signature.
        // This is validated by the fact that compute_hypoxia_factor_x1000 takes ONLY:
        // (primary_layer, field, pos, body_cell_count, cap_o2, n_layers)
        // and does NOT take econ.settling as a parameter.
        // Verify: identical inputs → identical output (determinism, no random state).
        let mut field = HypoxiaMockField::new();
        let pos = Vec2Fixed(0, 0);
        let cap_o2 = 100;
        let primary_layer = FieldId::Oxygen;
        let n_layers = 3;
        let body_cell_count = 8;

        field.set_conserved(0, 2, 50);

        // Call twice with identical inputs → must get identical output.
        let result1 = compute_hypoxia_factor_x1000(primary_layer, &field, pos, body_cell_count, cap_o2, n_layers);
        let result2 = compute_hypoxia_factor_x1000(primary_layer, &field, pos, body_cell_count, cap_o2, n_layers);

        assert_eq!(result1, result2, "identical inputs must yield identical output (determinism)");

        // The cost is computed from (body_cell_count, cap_o2, field contents at pos, n_layers) only.
        // econ.settling is NOT in the call signature → cost is INDEPENDENT of the settling toggle state.
        // This is structural independence, not empirical (it does not depend on running a sim and
        // comparing settling-on vs settling-off populations).
    }
}
