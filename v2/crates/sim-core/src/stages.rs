//! The 11 tick stages (0–10). Conserved-layer arithmetic is exact integer; every `eu` moved is
//! accounted in the [`EnergyLedger`] so the conservation residual stays EXACTLY 0 (R15).
//!
//! Stages that contend over a shared quantity resolve deterministically by **Entity-id order**:
//! Interactions (who eats a contested cell first) and BirthDeath (spawn/despawn order → deterministic
//! child Entity ids). Independent per-entity stages (Metabolism, Sense, Act) need no ordering.

use crate::*;
use bevy_ecs::prelude::*;

/// RNG salts — disjoint streams.
const SALT_MUT: u64 = 0x4D55_5400; // "MUT"

// ── Stage 0: SpatialRebuild — rebuild the Morton neighbor grid (R8). ───────────────────────────────
pub fn stage_spatial_rebuild(mut grid: ResMut<NeighborGrid>, q: Query<(Entity, &Position)>) {
    grid.clear();
    let mut ents: Vec<(u64, Entity, Vec2Fixed)> =
        q.iter().map(|(e, p)| (e.to_bits(), e, p.0)).collect();
    ents.sort_unstable_by_key(|x| x.0);
    for (_, e, p) in ents {
        grid.insert(p, e);
    }
}

// ── Stage 1: Sense — sample the resource gradient (read-old field) into warm Sensors. ──────────────
pub fn stage_sense(field: Res<FieldRes>, mut q: Query<(&Position, &Genome, &mut Sensors)>) {
    for (pos, g, mut s) in &mut q {
        let range = g.sense_range.max(1) as i64;
        let (gx, gz) = field.0.gradient_at(pos.0, range);
        s.gradient = Vec2Fixed(gx, gz);
        s.local_resource = field.0.amount_at(pos.0);
    }
}

// ── Stage 2: Brain — empty in Ф0 (chemotaxis is brain-less; real brains are M3). ───────────────────
pub fn stage_brain() {}

// ── Stage 3: Act — chemotaxis: desired velocity = sign(gradient) · move_speed → Intent. ────────────
//    No hidden fitness: moving toward food grants NO energy; it only pays off via actual feeding.
pub fn stage_act(mut q: Query<(&Sensors, &Genome, &mut Intent)>) {
    for (s, g, mut intent) in &mut q {
        let sp = g.move_speed as i64;
        intent.0 = Vec2Fixed(s.gradient.0.signum() * sp, s.gradient.1.signum() * sp);
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
    if !clock.tick.is_multiple_of(econ.metab_period) {
        return; // sub-tick period N (meta-constant; Ф0 N=1 so this always runs)
    }
    for (g, mut e) in &mut q {
        let cost = econ.base_metab
            + econ.k_size_metab * g.metab_units()
            + econ.k_move_cost * g.move_speed as i64
            + econ.k_sense_cost * g.sense_range as i64;
        // Can only dissipate what it has — energy never goes negative; death (energy 0) is in stage 7.
        let actual = cost.min(e.0.max(0));
        e.0 -= actual;
        ledger.dissipated += actual;
    }
}

// ── Stage 6: Interactions — feed: take from the field cell, convert at metabolism_eff. ─────────────
//    Ordered by Entity-id so contested cells resolve deterministically; integer transfer is exact.
pub fn stage_interactions(
    econ: Res<EconParams>,
    mut field: ResMut<FieldRes>,
    mut ledger: ResMut<EnergyLedger>,
    mut q: Query<(Entity, &Position, &Genome, &mut Energy)>,
) {
    let mut ents: Vec<(u64, Entity)> = q.iter().map(|(e, _, _, _)| (e.to_bits(), e)).collect();
    ents.sort_unstable_by_key(|x| x.0);
    for (_, e) in ents {
        let (_, pos, g, mut energy) = q.get_mut(e).expect("entity present");
        let got = field.0.take_at(pos.0, econ.u_max); // exact integer removal
        let gained = got * g.metabolism_eff as i64 / 256;
        let lost = got - gained; // conversion inefficiency → heat
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
) {
    repro.parents.clear();
    let mut ents: Vec<(u64, Entity)> = q.iter().map(|(e, _, _, _, _)| (e.to_bits(), e)).collect();
    ents.sort_unstable_by_key(|x| x.0);
    for (bits, e) in ents {
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
                genome.mutate(seed_fold(clock.seed, &[SALT_MUT, bits, clock.tick]));
            let species_c = *species;
            repro.parents.insert(bits);
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
            ));
        }
    }
}

// ── Stage 8: FieldScatter — commit staged contributions (→ next tick), regenerate (source), diffuse.
pub fn stage_field_scatter(mut field: ResMut<FieldRes>, mut ledger: ResMut<EnergyLedger>) {
    field.0.apply_scatter(); // R17: agent contributions land on the NEXT tick's field
    let injected = field.0.regenerate(); // explicit conservation SOURCE
    ledger.produced += injected;
    field.0.diffuse(); // conservative integer flux — Σ field invariant
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
            ],
            offspring,
        });
    }
    tel.population = ents.len() as i64;
    tel.field_total = field.0.total();
}

// ── Stage 10: Swap — double-buffer swap for Position + Velocity. ───────────────────────────────────
pub fn stage_swap(mut q: Query<(&mut Position, &PositionNext, &mut Velocity, &VelocityNext)>) {
    for (mut p, pn, mut v, vn) in &mut q {
        p.0 = pn.0;
        v.0 = vn.0;
    }
}
