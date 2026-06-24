//! Headless driver + golden-replay harness. Lives OUTSIDE the core (R1): the fixed-dt loop driver,
//! the wall clock, and the replay/golden machinery are here; `sim-core` has none of it.

use sim_core::Sim;

/// Run defaults for the M0 acceptance demo / golden.
pub const DEFAULT_SEED: u64 = 0xA11A_2A11; // "animata"
pub const DEFAULT_ENTITIES: u64 = 64;
pub const DEFAULT_TICKS: u64 = 256;

/// Fixed timestep dt = 1/64 s (doc 12 §4 — bevy FixedUpdate default), expressed in integer
/// microseconds so the loop driver does no floating-point either.
pub const DT_MICROS: u64 = 1_000_000 / 64; // 15_625 µs, exact

/// Golden-replay harness: from `(seed, n_entities)` and an EMPTY Phase-0 input log, produce the
/// per-tick state hash for `n_ticks` ticks. This is the replay carrier — `seed + log → hash[t]`.
pub fn run(seed: u64, n_entities: u64, n_ticks: u64) -> Vec<u64> {
    let mut sim = Sim::new(seed, n_entities);
    let mut hashes = Vec::with_capacity(n_ticks as usize);
    for _ in 0..n_ticks {
        sim.step();
        hashes.push(sim.state_hash());
    }
    hashes
}

/// The fixed-dt loop driver (R9): accumulate wall-frame time, drain it in fixed `dt` steps, capped
/// per frame to defuse the spiral of death. Integer-only. Returns the number of steps taken.
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
    /// Feed one wall frame of `frame_micros`; step the sim as many fixed ticks as have accumulated,
    /// up to the per-frame cap (leftover time stays in the accumulator).
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
    fn driver_caps_steps_against_spiral_of_death() {
        let mut sim = Sim::new(1, 4);
        let mut d = LoopDriver::default();
        // A huge frame would demand thousands of steps; the cap holds it to max_steps_per_frame.
        let steps = d.advance(10_000_000, &mut sim);
        assert_eq!(steps, 8);
    }

    #[test]
    fn driver_accumulates_subdt_frames() {
        let mut sim = Sim::new(1, 4);
        let mut d = LoopDriver::default();
        // Half a dt twice = one step total.
        assert_eq!(d.advance(DT_MICROS / 2, &mut sim), 0);
        assert_eq!(d.advance(DT_MICROS / 2 + 1, &mut sim), 1);
    }
}
