//! Headless driver + golden-replay harness. Lives OUTSIDE the core (R1): it wires the concrete
//! `world`/`fields` backends into `sim-core`, runs the fixed-dt loop, and enforces the always-on
//! energy-conservation invariant (R15 / F8 — active in `--release`, which is what CI runs).

use brain::FixedBrain;
use fields::{flux_k_from_alpha, CpuFieldStore};
use sim_core::{EconParams, MergeStrategy, Sim, SimConfig, Vec2Fixed, WorldView};
use world::NoiseWorld;

/// Fixed timestep dt = 1/64 s, integer microseconds (the loop driver does no float).
pub const DT_MICROS: u64 = 1_000_000 / 64;

// World/field tuning (documented cargo-parameters; re-pinning the golden after a change is cheap).
const HMAX: i64 = 16;
const RESOURCE_BASE: i64 = 120;
const REGEN_RATE: i64 = 6;
const M_FIELD: i64 = 1;
const WORLD_SALT: u64 = 0x5743_4C44; // "WCLD"
// Conserved flux diffusion: α = D·dt/dx² ∈ (0,¼]. Here α = 1/8, F = 16 → k = round(α·2^F) = 8192.
const FLUX_ALPHA_NUM: i64 = 1;
const FLUX_ALPHA_DEN: i64 = 8;
const FLUX_F: u32 = 16;
// Signal field: pheromone multiplicative decay λ per tick.
const SIGNAL_DECAY: f32 = 0.06;
/// Production thread count for the demo / fixed-N tests.
pub const DEFAULT_THREADS: usize = 4;

// L=3 scenario: layer 0 = energy-carrier (agents feed/excrete here only), layer 1 = nutrient,
// layer 2 = organics. Layers 1/2 regen/diffuse unconsumed in slice A (biology is slice B).
const L3_REGEN_RATES: [i64; 3] = [REGEN_RATE, 3, 1]; // l0=6 (same), l1=3 (medium), l2=1 (slow)
const L3_FLUX_ALPHA_NUMS: [i64; 3] = [1, 1, 1];
const L3_FLUX_ALPHA_DENS: [i64; 3] = [8, 16, 4]; // α=1/8 (l0=same), 1/16 (l1=slow), 1/4 (l2=fast)

/// Default Ф0 run on `threads` sim threads with the `Canonical` (correct) merge.
pub fn default_config(seed: u64) -> SimConfig {
    config_with(seed, DEFAULT_THREADS, MergeStrategy::Canonical)
}

/// A config with an explicit sim-thread count + merge strategy (the R14 test drives both).
pub fn config_with(seed: u64, sim_threads: usize, merge_strategy: MergeStrategy) -> SimConfig {
    SimConfig {
        seed,
        n_founders: 40,
        founder_energy: 1200,
        econ: EconParams::default(),
        sim_threads,
        merge_strategy,
        n_layers: 1,
    }
}

/// L=3 scenario config: three conserved layers (l0=energy-carrier, l1=nutrient, l2=organics).
/// Agents feed/excrete ONLY layer 0; layers 1/2 regen/diffuse unconsumed (biology is slice B).
pub fn l3_config(seed: u64) -> SimConfig {
    SimConfig { n_layers: 3, ..default_config(seed) }
}

/// Build a `Sim` with the noise world + the two-class field (conserved fixed-point + signal f32).
/// Per-cell caps come from `WorldView::resource` (float-noise-derived → arch-dependent trajectory).
/// `config.n_layers` selects L=1 (default) or L=3 (nutrient/organics scenario, A-4).
pub fn build_sim(config: SimConfig) -> Sim {
    let econ = config.econ;
    let world = NoiseWorld::new(econ.world_dim, HMAX, RESOURCE_BASE, config.seed ^ WORLD_SALT);
    let grid_w = econ.world_dim / M_FIELD;
    let n = (grid_w * grid_w) as usize;
    if config.n_layers == 1 {
        let mut caps = Vec::with_capacity(n);
        for cz in 0..grid_w {
            for cx in 0..grid_w {
                caps.push(world.resource(Vec2Fixed(cx * M_FIELD, cz * M_FIELD)));
            }
        }
        let flux_k = flux_k_from_alpha(FLUX_ALPHA_NUM, FLUX_ALPHA_DEN, FLUX_F);
        let field = CpuFieldStore::new(econ.world_dim, M_FIELD, caps, REGEN_RATE, flux_k, FLUX_F, SIGNAL_DECAY);
        // The integer fixed-point brain (M3) — one shared boxed backend for the whole population.
        Sim::new(config, Box::new(world), Box::new(field), Box::new(FixedBrain::new()))
    } else {
        assert_eq!(config.n_layers, 3, "only L=1 and L=3 supported");
        let mut caps0 = Vec::with_capacity(n);
        let mut caps1 = Vec::with_capacity(n);
        let caps2 = vec![60i64; n]; // organics: uniform cap
        for cz in 0..grid_w {
            for cx in 0..grid_w {
                let c = world.resource(Vec2Fixed(cx * M_FIELD, cz * M_FIELD));
                caps0.push(c);
                caps1.push(c * 2); // nutrient: 2× energy-carrier base
            }
        }
        let flux_ks: Vec<i64> = (0..3)
            .map(|i| flux_k_from_alpha(L3_FLUX_ALPHA_NUMS[i], L3_FLUX_ALPHA_DENS[i], FLUX_F))
            .collect();
        let field = CpuFieldStore::new_layered(
            econ.world_dim, M_FIELD,
            vec![caps0, caps1, caps2],
            L3_REGEN_RATES.to_vec(),
            flux_ks,
            FLUX_F,
            SIGNAL_DECAY,
        );
        Sim::new(config, Box::new(world), Box::new(field), Box::new(FixedBrain::new()))
    }
}

/// Perf-gate bench scenario config: `world_dim=128` (4× area vs default 64×64) supports a large
/// sustained population so an O(N²) regression provably breaches the per-entity work bounds (D1a/F8).
/// `n_founders` pre-populates the world — the ecosystem is resource-rich enough that no mass starvation
/// occurs on tick 1 (128×128 cells, same per-cell regen, carrying capacity ≈400+ creatures).
pub fn bench_config(seed: u64, n_founders: u64) -> SimConfig {
    let econ = EconParams { world_dim: 128, ..EconParams::default() };
    SimConfig { seed, n_founders, founder_energy: 1200, econ, sim_threads: DEFAULT_THREADS, merge_strategy: MergeStrategy::Canonical, n_layers: 1 }
}

/// Build a sim on the perf-gate bench scale (world_dim=128, `n_founders` pre-populated).
pub fn build_sim_bench(seed: u64, n_founders: u64) -> Sim {
    build_sim(bench_config(seed, n_founders))
}

/// Golden-replay harness: `(config) → per-tick state hash`, with the always-on guards firing every
/// tick (active in `--release`, F8): exact energy conservation (R15) AND the signal NaN/Inf guard.
pub fn run(config: SimConfig, ticks: u64) -> Vec<u64> {
    let mut sim = build_sim(config);
    let mut hashes = Vec::with_capacity(ticks as usize);
    for _ in 0..ticks {
        sim.step();
        let residual = sim.conservation_residual();
        assert_eq!(residual, 0, "ENERGY CONSERVATION VIOLATED at tick {}: residual={residual}", sim.tick());
        assert!(sim.signal_finite(), "SIGNAL NaN/Inf at tick {}", sim.tick());
        hashes.push(sim.state_hash());
    }
    hashes
}

/// Per-tick CONSERVED-field hash over all layers (the R14 subject). Integer ⇒ arch-independent as
/// a relative comparison. `config.n_layers` selects L=1 or L=3.
pub fn run_conserved_hashes(config: SimConfig, ticks: u64) -> Vec<u64> {
    let mut sim = build_sim(config);
    let mut hashes = Vec::with_capacity(ticks as usize);
    for _ in 0..ticks {
        sim.step();
        hashes.push(sim.conserved_field_hash());
    }
    hashes
}

/// The fixed-dt loop driver (R9): accumulate wall-frame time, drain in fixed `dt` steps, capped per
/// frame against the spiral of death. Integer-only.
pub struct LoopDriver {
    acc_micros: u64,
    dt_micros: u64,
    max_steps_per_frame: u32,
}

impl Default for LoopDriver {
    fn default() -> Self {
        Self { acc_micros: 0, dt_micros: DT_MICROS, max_steps_per_frame: 8 }
    }
}

impl LoopDriver {
    pub fn advance(&mut self, frame_micros: u64, sim: &mut Sim) -> u32 {
        self.acc_micros += frame_micros;
        let mut steps = 0;
        while self.acc_micros >= self.dt_micros && steps < self.max_steps_per_frame {
            sim.step();
            self.acc_micros -= self.dt_micros;
            steps += 1;
        }
        steps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_caps_steps() {
        let mut sim = build_sim(default_config(1));
        let mut d = LoopDriver::default();
        assert_eq!(d.advance(10_000_000, &mut sim), 8);
    }

    /// R27 guard: the fixed `dt` is NEVER scaled by a time-multiplier in the v2 headless core.
    /// The only valid tempo controls are `EconParams::brain_period` (K) and `metab_period` (N) —
    /// both are integer divisors of the tick counter. Max-speed headless is the default (no vsync).
    /// A time-scale multiplier would violate determinism and break the R20 multi-rate contract.
    #[test]
    fn v2_r27_dt_is_not_timescaled() {
        // dt is the canonical fixed step: 1/64 s = 15625 µs. It must never be scaled.
        const EXPECTED_DT_MICROS: u64 = 1_000_000 / 64; // 15625
        assert_eq!(
            DT_MICROS, EXPECTED_DT_MICROS,
            "R27: DT_MICROS must be the exact canonical 1/64 s fixed step, never time-scaled"
        );
        // LoopDriver caps at max_steps_per_frame (8) regardless of how large the accumulated time is.
        // That cap is the only acceleration mechanism — it is NOT a dt-scaling bypass.
        let mut sim = build_sim(default_config(42));
        let mut d = LoopDriver::default();
        let steps = d.advance(10_000_000, &mut sim); // 10 s of accumulated time → still capped at 8
        assert_eq!(
            steps, 8,
            "R27: LoopDriver caps at max_steps_per_frame, never at a scaled dt"
        );
    }
}
