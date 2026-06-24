//! Headless driver + golden-replay harness. Lives OUTSIDE the core (R1): it wires the concrete
//! `world`/`fields` backends into `sim-core`, runs the fixed-dt loop, and enforces the always-on
//! energy-conservation invariant (R15 / F8 — active in `--release`, which is what CI runs).

use fields::CpuResourceField;
use sim_core::{EconParams, Sim, SimConfig, Vec2Fixed, WorldView};
use world::NoiseWorld;

/// Fixed timestep dt = 1/64 s, integer microseconds (the loop driver does no float).
pub const DT_MICROS: u64 = 1_000_000 / 64;

// World/field tuning (documented cargo-parameters; re-pinning the golden after a change is cheap).
const HMAX: i64 = 16;
const RESOURCE_BASE: i64 = 120;
const REGEN_RATE: i64 = 6;
const DIFFUSE_SHIFT: u32 = 3;
const M_FIELD: i64 = 1;
const WORLD_SALT: u64 = 0x5743_4C44; // "WCLD"

/// Default Ф0 run — founders, energy, economy. Tuned so the population is bounded and persistent.
pub fn default_config(seed: u64) -> SimConfig {
    SimConfig { seed, n_founders: 40, founder_energy: 1200, econ: EconParams::default() }
}

/// Build a `Sim` with the noise world + conserved resource field wired in. The field's per-cell caps
/// come from `WorldView::resource` (float-noise-derived → arch-dependent), which is what makes the
/// trajectory arch-dependent (→ arm64 golden).
pub fn build_sim(config: SimConfig) -> Sim {
    let econ = config.econ;
    let world = NoiseWorld::new(econ.world_dim, HMAX, RESOURCE_BASE, config.seed ^ WORLD_SALT);
    let grid_w = econ.world_dim / M_FIELD;
    let mut caps = Vec::with_capacity((grid_w * grid_w) as usize);
    for cz in 0..grid_w {
        for cx in 0..grid_w {
            caps.push(world.resource(Vec2Fixed(cx * M_FIELD, cz * M_FIELD)));
        }
    }
    let field = CpuResourceField::new(econ.world_dim, M_FIELD, caps, REGEN_RATE, DIFFUSE_SHIFT);
    Sim::new(config, Box::new(world), Box::new(field))
}

/// Golden-replay harness: `(config) → per-tick state hash` for `ticks` ticks, with the always-on
/// energy-conservation assertion firing every tick (R15, active in release).
pub fn run(config: SimConfig, ticks: u64) -> Vec<u64> {
    let mut sim = build_sim(config);
    let mut hashes = Vec::with_capacity(ticks as usize);
    for _ in 0..ticks {
        sim.step();
        let residual = sim.conservation_residual();
        assert_eq!(
            residual, 0,
            "ENERGY CONSERVATION VIOLATED at tick {}: residual={}",
            sim.tick(),
            residual
        );
        hashes.push(sim.state_hash());
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
}
