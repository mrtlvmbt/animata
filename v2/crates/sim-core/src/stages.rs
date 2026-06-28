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

/// RNG salts — disjoint streams.
const SALT_MUT: u64 = 0x4D55_5400; // "MUT"

// Stage 0 (SpatialRebuild) REMOVED (M1/F2): the NeighborGrid was rebuilt every tick but never
// queried by any stage — dead per-tick work. Removed until a real neighbour-coupled consumer lands.

// ── Stage 1: Sense — read the conserved resource field (version t): integer gradient + local amount.
//    Signal pheromone gradient is intentionally NOT fed to the integer brain in M3 (see stage_brain);
//    the dead per-tick compute was removed (M3/F3). Signal still contributes to state_hash via
//    signal_hash(), keeping the golden arm64-pinned. ───────────────────────────────────────────────
pub fn stage_sense(field: Res<FieldRes>, mut q: Query<(&Position, &Genome, &mut Sensors)>) {
    for (pos, g, mut s) in &mut q {
        let range = g.sense_range.max(1) as i64;
        let layer = g.uptake_layer as usize; // B-2: sense the layer the agent eats from
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
pub fn stage_metabolism(
    econ: Res<EconParams>,
    clock: Res<SimClock>,
    mut ledger: ResMut<EnergyLedger>,
    mut q: Query<(&Genome, &mut Energy)>,
) {
    let n = econ.metab_period.max(1);
    if !clock.tick.is_multiple_of(n) {
        return; // multi-rate metabolism (D-Brain-4): runs every N ticks, GLOBAL phase.
    }
    for (g, mut e) in &mut q {
        // Charge ×N — a lump standing in for the N base ticks since the last metabolism tick, so the
        // economy stays ≈invariant to N and conservation is exact (R15).
        let cost = (econ.base_metab
            + econ.k_size_metab * g.metab_units()
            + econ.k_move_cost * g.move_speed as i64
            + econ.k_sense_cost * g.sense_range as i64)
            * n as i64;
        // Can only dissipate what it has — energy never goes negative; death (energy 0) is in stage 7.
        let actual = cost.min(e.0.max(0));
        e.0 -= actual;
        ledger.dissipated += actual;
    }
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

// ── Stage 6: Interactions — feed: proportional deficit rationing (B-3). ─────────────────────────
//    At a deficit cell (Σ demand > R_cell) each agent's grant is `U_i·R_cell / Σ U_j` (integer
//    truncating). Non-deficit cells grant each agent its full Monod demand.
//    Algorithm — ONE gather pass (no double archetype lookup), then sort by (cell×4+layer, entity)
//    so same-(cell,layer) contestants are contiguous. Two cheap walks (Σ then grant) followed by one
//    get_mut apply loop.  Order-independent: Σ is associative; grants depend only on cell totals.
pub fn stage_interactions(
    econ: Res<EconParams>,
    mut field: ResMut<FieldRes>,
    mut ledger: ResMut<EnergyLedger>,
    mut q: Query<(Entity, &Position, &Genome, &mut Energy)>,
    #[cfg(feature = "perf")] mut wc: ResMut<WorkCounters>,
) {
    // 1. Gather: one read per entity (Monod demand). No `conserved_take` yet.
    //    Sort key = cell_index * 4 + layer (B-2: layer ∈ 0..4); secondary = entity_bits.
    struct Contestant {
        cell_layer: usize, // cell_index * 4 + layer — the group key (B-2: layer ∈ 0..4)
        entity_bits: u64,
        entity: Entity,
        pos: Vec2Fixed,
        layer: usize,
        demand: i64,
    }
    let mut contestants: Vec<Contestant> = q.iter().map(|(e, pos, g, _)| {
        let layer = g.uptake_layer as usize;
        let r = field.0.conserved_at(pos.0, layer);
        let demand = monod_demand(econ.u_max, econ.km, r);
        let cell = field.0.cell_index(pos.0);
        Contestant {
            cell_layer: cell * 4 + layer,
            entity_bits: e.to_bits(),
            entity: e,
            pos: pos.0,
            layer,
            demand,
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
        let r_cell = field.0.conserved_at(contestants[run_start].pos, contestants[run_start].layer);
        // Σ demand over this run.
        let sigma: i64 = contestants[run_start..run_end].iter().map(|c| c.demand).sum();
        if sigma <= r_cell {
            // No deficit: each agent gets its full Monod demand.
            for i in run_start..run_end {
                grants[i] = contestants[i].demand;
            }
        } else if r_cell == 0 {
            // Empty cell: no grants (all zeros already).
        } else {
            // Deficit: proportional ration — ⌊U_i · R_cell / Σ⌋.
            for i in run_start..run_end {
                grants[i] = contestants[i].demand * r_cell / sigma;
            }
        }
        run_start = run_end;
    }

    // 3. Apply grants: ONE get_mut per entity (no second archetype scan).
    //    `conserved_take` is called for the GRANT amount (may be < demand at deficit cells).
    for (i, c) in contestants.iter().enumerate() {
        #[cfg(feature = "perf")]
        { wc.field_takes += 1; }
        let (_, _, g, mut energy) = q.get_mut(c.entity).expect("entity present");
        let got = field.0.conserved_take(c.pos, grants[i], c.layer);
        let gained = got * g.metabolism_eff as i64 / 256;
        let lost = got - gained;
        energy.0 += gained;
        ledger.dissipated += lost;
    }
}

// ── Stage 7: BirthDeath — division (energy split) + death, via the command buffer (sync point). ────
pub fn stage_birth_death(
    econ: Res<EconParams>,
    clock: Res<SimClock>,
    mut ledger: ResMut<EnergyLedger>,
    mut repro: ResMut<ReproEvents>,
    mut commands: Commands,
    mut q: Query<(Entity, &Position, &mut Energy, &Genome, &SpeciesId)>,
    #[cfg(feature = "perf")] mut wc: ResMut<WorkCounters>,
) {
    repro.parents.clear();
    let mut ents: Vec<(u64, Entity)> = q.iter().map(|(e, _, _, _, _)| (e.to_bits(), e)).collect();
    ents.sort_unstable_by_key(|x| x.0);
    for (bits, e) in ents {
        #[cfg(feature = "perf")]
        { wc.birth_death_iters += 1; }
        let (_, pos, mut energy, genome, species) = q.get_mut(e).expect("entity present");
        if energy.0 <= 0 {
            // Death (starvation): energy is exactly 0 → nothing to recycle, conservation intact.
            commands.entity(e).despawn();
            continue;
        }
        if energy.0 >= genome.repro_threshold as i64 && energy.0 >= econ.e_cell + econ.c_div {
            // Division: child stock e_cell stays in the system (the child), c_div dissipated.
            // Δenergy = −(e_cell + c_div) + e_cell(child) + c_div(dissipated) = 0  (conserved).
            energy.0 -= econ.e_cell + econ.c_div;
            ledger.dissipated += econ.c_div;
            let pos_c = *pos;
            let child_genome =
                genome.mutate(seed_fold(clock.seed, &[SALT_MUT, bits, clock.tick]), econ.n_layers);
            let species_c = *species;
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
            commands.spawn((
                Position(pos_c.0),
                PositionNext(pos_c.0),
                Velocity::default(),
                VelocityNext::default(),
                Energy(econ.e_cell),
                child_genome,
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
    q: Query<(Entity, &Genome)>,
) {
    tel.samples.clear();
    let mut ents: Vec<(u64, Genome)> = q.iter().map(|(e, g)| (e.to_bits(), *g)).collect();
    ents.sort_unstable_by_key(|x| x.0);
    for (bits, g) in &ents {
        let offspring = u32::from(repro.parents.contains(bits));
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
        });
    }
    tel.population = ents.len() as i64;
    tel.field_total = field.0.conserved_total_all();
    // Signal-field metric (R25) — serial total concentration; never feeds the tick.
    tel.signal_total = field.0.signal_total();
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
    use super::monod_demand;

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
}
