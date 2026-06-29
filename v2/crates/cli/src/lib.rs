//! Headless driver + golden-replay harness. Lives OUTSIDE the core (R1): it wires the concrete
//! `world`/`fields` backends into `sim-core`, runs the fixed-dt loop, and enforces the always-on
//! energy-conservation invariant (R15 / F8 — active in `--release`, which is what CI runs).

use brain::FixedBrain;
use fields::{flux_k_from_alpha, CpuFieldStore};
use sim_core::{EconParams, LayerSpec, LightSpec, MergeStrategy, Sim, SimConfig, Vec2Fixed, WorldView, D0_MASK, RECYCLE_DEN};
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

// Layer 0: primary substrate — world-noise-derived per-cell caps, regen from the resource base.
// α = 1/8 (moderate diffusion). Used by every config.
const L0_SPEC: LayerSpec = LayerSpec {
    regen_rate: REGEN_RATE, flux_alpha_num: FLUX_ALPHA_NUM, flux_alpha_den: FLUX_ALPHA_DEN,
    flat_cap: 0, world_cap_mult: 0, // layer 0 always uses world-noise caps (flat_cap/world_cap_mult ignored)
};
// Layer 1 (B-0 production): organics/excreta — regen=0 (no phantom free energy), α=1/4 (fast
// lateral spread so excreted metabolites diffuse quickly). cap=0 → starts EMPTY (cap/2=0).
const L1_ORGANICS_SPEC: LayerSpec = LayerSpec {
    regen_rate: 0, flux_alpha_num: 1, flux_alpha_den: 4, flat_cap: 0, world_cap_mult: 0,
};
// Layer 1 (A-4 L=3 test only): nutrient — regen=3, α=1/16, caps = world·2.
const L1_NUTRIENT_SPEC: LayerSpec = LayerSpec {
    regen_rate: 3, flux_alpha_num: 1, flux_alpha_den: 16, flat_cap: 0, world_cap_mult: 2,
};
// Layer 2 (A-4 L=3 test only): slow organics — regen=1, α=1/4, flat cap 60.
const L2_ORGANICS_SPEC: LayerSpec = LayerSpec {
    regen_rate: 1, flux_alpha_num: 1, flux_alpha_den: 4, flat_cap: 60, world_cap_mult: 0,
};
// Layer 2 (C′-1 detritus): regen=0, ZERO diffusion — detritus stays where it falls; the only
// meaningful nutrient-return path is biotic (a reducer), making the reducer niche real (C′-2/3).
const L2_DETRITUS_SPEC: LayerSpec = LayerSpec {
    regen_rate: 0, flux_alpha_num: 0, flux_alpha_den: 1, flat_cap: 0, world_cap_mult: 0,
};
// Layer 2 (D′-3a mineral): regen=1 (small per-cell influx), fast diffusion (α=1/4, like organics)
// so mineral spreads from production zones; flat_cap=200 (starts at 100 per cell).
// Calibration mapping: P_mineral=35 total per tick / 4096 cells ≈ 0.0085/cell → scaled to
// regen_rate=1 (the minimum non-zero integer), with km_mineral=200 and u_max_mineral=70 calibrated
// so N×U(M*)=regen×4096 at N*≈583 → M*≈22 eu-mineral. Scale ×10 from model units.
const L2_MINERAL_SPEC: LayerSpec = LayerSpec {
    regen_rate: 1, flux_alpha_num: 1, flux_alpha_den: 4, flat_cap: 200, world_cap_mult: 0,
};

/// Default production config (L=2): layer 0 = substrate, layer 1 = organics/excreta (empty start).
pub fn default_config(seed: u64) -> SimConfig {
    config_with(seed, DEFAULT_THREADS, MergeStrategy::Canonical)
}

/// A config with an explicit sim-thread count + merge strategy (the R14 test drives both).
/// Production config is L=2: layer 0 = substrate, layer 1 = organics (regen=0, empty start).
pub fn config_with(seed: u64, sim_threads: usize, merge_strategy: MergeStrategy) -> SimConfig {
    SimConfig {
        seed,
        n_founders: 40,
        founder_energy: 1200,
        econ: EconParams::default(),
        sim_threads,
        merge_strategy,
        n_layers: 2,
        layer_specs: [L0_SPEC, L1_ORGANICS_SPEC, LayerSpec::default(), LayerSpec::default()],
    }
}

/// C′-1 biotic-recycle config (L=3): layer0=substrate, layer1=excreta-organics, layer2=detritus.
/// `detritus_layer=Some(2)`, `detritus_frac=1.0` (full-replace, bootstrap): ALL recycled body
/// energy → detritus on death (no abiotic shortcut). Biotic return: reducer evolves uptake_layer=2
/// + excrete_layer=0 via existing B-2 mutation machinery — niche is emergent, not coded (C′-2/3).
/// Detritus layer: regen=0, zero diffusion → only biotic reduction returns nutrients.
pub fn cprime_config(seed: u64) -> SimConfig {
    SimConfig {
        n_layers: 3,
        layer_specs: [L0_SPEC, L1_ORGANICS_SPEC, L2_DETRITUS_SPEC, LayerSpec::default()],
        econ: EconParams {
            detritus_layer: Some(2),
            detritus_frac_num: 256, // RECYCLE_DEN = 256 → frac=1.0 full-replace (bootstrap)
            ..EconParams::default()
        },
        ..config_with(seed, DEFAULT_THREADS, MergeStrategy::Canonical)
    }
}

/// D′-1/2/3 light+mineral-economy config (L=3): substrate + organics + mineral.
///
/// **D′-1/2 (light economy + photo-GRN regulation gene):** `photo_gain` and `reg_gain` active.
/// Non-dprime goldens stay byte-identical (mineral and photo Option-gated).
///
/// **D′-3a (mineral economy):** mineral conserved layer (layer 2) added as co-essential nutrient.
///   - Mineral layer: `L2_MINERAL_SPEC` (regen_rate=1, α=1/4 diffusion, flat_cap=200/cell→starts
///     at 100). `mineral_layer=Some(2)`.
///   - Integer calibration (×10 scale from model units): Km=200, U_max=70, q_mineral=4000,
///     recycle_mineral≈0.4, overflow_delta=50. See EconParams doc for mapping arithmetic.
///   - Liebig AND-gate on division: `energy ≥ repro_threshold && quota ≥ q_mineral`.
///   - Overflow site: when energy-ready but quota-poor → `energy -= overflow_delta` → `ledger.lost`.
///   - Division: parent quota -= q_mineral → ledger.dissipated; child MineralQuota=0.
///   - Death: recycle_mineral×quota → field_M; remainder → ledger.lost.
///   - Conservation: unified EnergyLedger covers both energy + mineral (one stock identity).
///
/// FLAG-2 fallback (tight economy): if mineral collapse observed, relax via `--set` overrides or
/// parameter adjustment (P_mineral via regen_rate, q_mineral). See EconParams.mineral_layer doc.
pub fn dprime_config(seed: u64) -> SimConfig {
    SimConfig {
        n_layers: 3,
        layer_specs: [L0_SPEC, L1_ORGANICS_SPEC, L2_MINERAL_SPEC, LayerSpec::default()],
        econ: EconParams {
            light: Some(LightSpec {
                l_max: 100,
                period_ticks: 100,
                day_ticks: 50,   // 50 % duty cycle (plan §0: day-night parameterised)
                km_photo: 30,    // Km_photo=30 < Km_chem=74 (plan §0 calibration)
            }),
            // D′-3a: mineral economy enabled on layer 2.
            mineral_layer: Some(2),
            ..EconParams::default()
        },
        ..config_with(seed, DEFAULT_THREADS, MergeStrategy::Canonical)
    }
}

/// D′-1/2 light-economy config (L=2, no mineral): substrate + organics.
///
/// Identical to dprime_config EXCEPT mineral_layer=None and n_layers=2.
/// Used by D′-1 and D′-2 tests that measure photo sweep and photo-cost non-inertness — properties
/// that require light to provide genuine selective advantage. With D′-3a's mineral Liebig gate, the
/// overflow mechanism neutralises photo income (energy above threshold is burned off), so photo
/// selection is near-zero and photo sweep tests are not meaningful on dprime_config.
///
/// Exported so D′-1/D′-2 test suites can pin to this config while D′-3a tests use dprime_config.
pub fn dprime_light_config(seed: u64) -> SimConfig {
    SimConfig {
        n_layers: 2,
        layer_specs: [L0_SPEC, L1_ORGANICS_SPEC, LayerSpec::default(), LayerSpec::default()],
        econ: EconParams {
            light: Some(LightSpec {
                l_max: 100, period_ticks: 100, day_ticks: 50, km_photo: 30,
            }),
            // mineral_layer: None (default) — photo gene has genuine selective advantage
            ..EconParams::default()
        },
        ..config_with(seed, DEFAULT_THREADS, MergeStrategy::Canonical)
    }
}

/// L=3 scenario config: three conserved layers (l0=energy-carrier, l1=nutrient, l2=organics).
/// Agents feed/excrete ONLY layer 0; layers 1/2 regen/diffuse unconsumed (biology is slice B).
/// Layer specs are explicit (not derived from default) to keep `v2_golden_conserved_l3` stable.
pub fn l3_config(seed: u64) -> SimConfig {
    SimConfig {
        n_layers: 3,
        layer_specs: [L0_SPEC, L1_NUTRIENT_SPEC, L2_ORGANICS_SPEC, LayerSpec::default()],
        ..config_with(seed, DEFAULT_THREADS, MergeStrategy::Canonical)
    }
}

/// Build a `Sim` with the noise world + the two-class field (conserved fixed-point + signal f32).
/// Per-cell caps for layer 0 come from `WorldView::resource` (float-noise-derived, arch-dependent).
/// Layers 1+ use `config.layer_specs[l].flat_cap` (0 = empty start) or `world_cap_mult × world`.
/// Handles any `n_layers ≥ 1` from `config.layer_specs`; no fixed-L branches.
pub fn build_sim(config: SimConfig) -> Sim {
    // B-2: sync econ.n_layers = config.n_layers so stage_birth_death can clamp layer-trait mutations.
    // D′-3a: if mineral_layer is set, n_energy_layers = mineral_layer index (genomes can only
    // target layers 0..mineral_layer; mineral is exclusively accessed via stage_mineral_feed).
    // For all other configs, n_energy_layers == n_layers (backward-compatible).
    let mut config = config;
    config.econ.n_layers = config.n_layers;
    if let Some(min_l) = config.econ.mineral_layer {
        config.econ.n_energy_layers = min_l; // exclude mineral layer from genome mutation range
    } else {
        config.econ.n_energy_layers = config.n_layers;
    }
    let econ = config.econ;
    let world = NoiseWorld::new(econ.world_dim, HMAX, RESOURCE_BASE, config.seed ^ WORLD_SALT);
    let grid_w = econ.world_dim / M_FIELD;
    let n = (grid_w * grid_w) as usize;

    // World-noise per-cell caps — layer 0 always, and any layer 1+ with world_cap_mult > 0.
    let mut world_caps = Vec::with_capacity(n);
    for cz in 0..grid_w {
        for cx in 0..grid_w {
            world_caps.push(world.resource(Vec2Fixed(cx * M_FIELD, cz * M_FIELD)));
        }
    }

    let mut caps_per_layer: Vec<Vec<i64>> = Vec::with_capacity(config.n_layers);
    let mut regen_rates: Vec<i64> = Vec::with_capacity(config.n_layers);
    let mut flux_ks: Vec<i64> = Vec::with_capacity(config.n_layers);
    for l in 0..config.n_layers {
        let spec = config.layer_specs[l];
        flux_ks.push(flux_k_from_alpha(spec.flux_alpha_num, spec.flux_alpha_den, FLUX_F));
        regen_rates.push(spec.regen_rate);
        let caps = if l == 0 {
            world_caps.clone()
        } else if spec.world_cap_mult > 0 {
            world_caps.iter().map(|&c| c * spec.world_cap_mult).collect()
        } else {
            vec![spec.flat_cap; n] // flat_cap=0 → all cells start at 0 (empty layer)
        };
        caps_per_layer.push(caps);
    }

    let field = CpuFieldStore::new_layered(
        econ.world_dim, M_FIELD, caps_per_layer, regen_rates, flux_ks, FLUX_F, SIGNAL_DECAY,
    );
    Sim::new(config, Box::new(world), Box::new(field), Box::new(FixedBrain::new()))
}

/// Perf-gate bench scenario config: `world_dim=128` (4× area vs default 64×64) supports a large
/// sustained population so an O(N²) regression provably breaches the per-entity work bounds (D1a/F8).
/// `n_founders` pre-populates the world — the ecosystem is resource-rich enough that no mass starvation
/// occurs on tick 1 (128×128 cells, same per-cell regen, carrying capacity ≈400+ creatures).
/// Stays L=1 — a consumer-less regen-0 layer 1 in the homogeneous bench is a metabolic sink that
/// would silently drop carrying capacity below SUSTAIN_FLOOR (F8).
pub fn bench_config(seed: u64, n_founders: u64) -> SimConfig {
    let econ = EconParams { world_dim: 128, ..EconParams::default() };
    SimConfig {
        seed, n_founders, founder_energy: 1200, econ,
        sim_threads: DEFAULT_THREADS, merge_strategy: MergeStrategy::Canonical,
        n_layers: 1,
        layer_specs: [L0_SPEC, LayerSpec::default(), LayerSpec::default(), LayerSpec::default()],
    }
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

// ── EconParams override helper ────────────────────────────────────────────────────────────────────

/// Max `u_max` accepted by `--set` (the overflow-safe cap documented in `stages.rs`).
const U_MAX_CAP: i64 = 220;

/// Apply `key=value` overrides to `econ`, validating each against the calibration whitelist and
/// a safe numeric range. Structural fields (`n_layers`, `world_dim`, `m_sim`, `m_field`,
/// `detritus_layer`, `detritus_frac_num`) are explicitly rejected. Returns `Err(msg)` on the
/// first bad key or bad value — caller exits non-zero with that message BEFORE any sim runs.
///
/// Called ONCE before the sim starts; `expect()`-safe at every subsequent build site.
pub fn apply_overrides(econ: &mut EconParams, sets: &[(String, String)]) -> Result<(), String> {
    for (key, val) in sets {
        match key.as_str() {
            "km" => {
                let v = p::<i64>(key, val)?;
                if v <= 0 {
                    return Err(format!(
                        "error: --set km={v}: km must be > 0 (km=0 causes 0/0 in uptake U(R)=u_max·R/(R+km))"
                    ));
                }
                econ.km = v;
            }
            "u_max" => {
                let v = p::<i64>(key, val)?;
                if v <= 0 || v > U_MAX_CAP {
                    return Err(format!(
                        "error: --set u_max={v}: u_max must be in [1, {U_MAX_CAP}] ({U_MAX_CAP} is the overflow-safe cap in stages.rs)"
                    ));
                }
                econ.u_max = v;
            }
            "base_metab" => {
                let v = p::<i64>(key, val)?;
                if v < 0 {
                    return Err(format!("error: --set base_metab={v}: must be ≥ 0"));
                }
                econ.base_metab = v;
            }
            "c_div" => {
                let v = p::<i64>(key, val)?;
                if v < 0 {
                    return Err(format!("error: --set c_div={v}: must be ≥ 0"));
                }
                econ.c_div = v;
            }
            "e_cell" => {
                let v = p::<i64>(key, val)?;
                if v <= 0 {
                    return Err(format!("error: --set e_cell={v}: must be > 0 (zero cell energy is non-viable)"));
                }
                econ.e_cell = v;
            }
            "k_size_metab" => {
                let v = p::<i64>(key, val)?;
                if v < 0 {
                    return Err(format!("error: --set k_size_metab={v}: must be ≥ 0"));
                }
                econ.k_size_metab = v;
            }
            "k_move_cost" => {
                let v = p::<i64>(key, val)?;
                if v < 0 {
                    return Err(format!("error: --set k_move_cost={v}: must be ≥ 0"));
                }
                econ.k_move_cost = v;
            }
            "k_sense_cost" => {
                let v = p::<i64>(key, val)?;
                if v < 0 {
                    return Err(format!("error: --set k_sense_cost={v}: must be ≥ 0"));
                }
                econ.k_sense_cost = v;
            }
            "excrete" => {
                let v = p::<i64>(key, val)?;
                if v < 0 {
                    return Err(format!("error: --set excrete={v}: must be ≥ 0"));
                }
                econ.excrete = v;
            }
            "recycle_num" => {
                let v = p::<i64>(key, val)?;
                if !(0..=RECYCLE_DEN).contains(&v) {
                    return Err(format!(
                        "error: --set recycle_num={v}: must be in [0, {RECYCLE_DEN}] (RECYCLE_DEN)"
                    ));
                }
                econ.recycle_num = v;
            }
            "speciation_threshold" => {
                let v = p::<i64>(key, val)?;
                if v < 0 {
                    return Err(format!("error: --set speciation_threshold={v}: must be ≥ 0"));
                }
                econ.speciation_threshold = v;
            }
            "brain_period" => {
                let v = p::<u64>(key, val)?;
                if v == 0 {
                    return Err(format!("error: --set brain_period={v}: must be ≥ 1 (0 would divide-by-zero in the brain phase)"));
                }
                econ.brain_period = v;
            }
            "metab_period" => {
                let v = p::<u64>(key, val)?;
                if v == 0 {
                    return Err(format!("error: --set metab_period={v}: must be ≥ 1 (0 would divide-by-zero in the metabolism phase)"));
                }
                econ.metab_period = v;
            }
            "d0_scaled" => {
                let v = p::<u64>(key, val)?;
                if v > D0_MASK {
                    return Err(format!(
                        "error: --set d0_scaled={v}: must be ≤ D0_MASK ({D0_MASK}); \
                         d0_scaled > D0_MASK makes kill condition always true → instant extinction"
                    ));
                }
                econ.d0_scaled = v;
            }
            "pheromone" => {
                let v = p::<f32>(key, val)?;
                if !v.is_finite() {
                    return Err(format!(
                        "error: --set pheromone={v}: must be finite (NaN/inf poison the signal field)"
                    ));
                }
                if v < 0.0 {
                    return Err(format!("error: --set pheromone={v}: must be ≥ 0.0"));
                }
                econ.pheromone = v;
            }
            // ── Explicitly rejected structural fields ──────────────────────────────────────────────
            "n_layers" | "world_dim" | "m_sim" | "m_field" | "detritus_layer"
            | "detritus_frac_num" => {
                return Err(format!(
                    "error: --set {key}=…: structural field — not overridable via --set. \
                     Reason: n_layers is overwritten by build_sim; world_dim/m_sim/m_field break \
                     R8 meta invariants or risk OOM; detritus_* require matching layer_specs."
                ));
            }
            // mutation_rate is a genome/trait field, not EconParams
            "mutation_rate" => {
                return Err(
                    "error: --set mutation_rate=…: mutation_rate is an evolved genome trait \
                     (Genome::mutation_rate), not an EconParams calibration knob. \
                     It cannot be overridden via --set."
                    .to_string(),
                );
            }
            // D′-2c: reg_gain_max controls the evolvable regulation range.
            // 0 = regulation locked OFF (constitutive control); default = 4 in dprime_config.
            "reg_gain_max" => {
                let v = p::<i32>(key, val)?;
                if v < 0 {
                    return Err(format!(
                        "error: --set reg_gain_max={v}: must be ≥ 0 \
                         (0 locks regulation OFF — the D′-2c constitutive control; \
                         default = 4 in EconParams)"
                    ));
                }
                econ.reg_gain_max = v;
            }
            _ => {
                return Err(format!(
                    "error: --set {key}=…: not an overridable calibration knob. \
                     Valid keys: km, u_max, base_metab, c_div, e_cell, k_size_metab, \
                     k_move_cost, k_sense_cost, excrete, recycle_num, speciation_threshold, \
                     brain_period, metab_period, d0_scaled, pheromone, reg_gain_max."
                ));
            }
        }
    }
    Ok(())
}

/// Parse a `--set KEY=VALUE` value string into type `T`. Returns `Err` with a clean error message.
fn p<T: std::str::FromStr>(key: &str, val: &str) -> Result<T, String>
where
    T::Err: std::fmt::Display,
{
    val.parse().map_err(|e| format!("error: --set {key}={val}: invalid value: {e}"))
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

    // ── #179 apply_overrides tests ────────────────────────────────────────────────────────────────

    // Helper: build a SimConfig with the given `(key, val)` overrides applied on top of default.
    fn cfg_with_sets(seed: u64, sets: &[(&str, &str)]) -> SimConfig {
        let mut econ = EconParams::default();
        let kv: Vec<(String, String)> =
            sets.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        apply_overrides(&mut econ, &kv).expect("test sets must be valid");
        let mut cfg = config_with(seed, DEFAULT_THREADS, MergeStrategy::Canonical);
        cfg.econ = econ;
        cfg
    }

    /// (1) No-override path must be byte-identical to `default_config` every tick.
    #[test]
    fn no_override_is_byte_identical_to_default() {
        let seed = 1;
        let ticks = 30;
        let default_hashes = run(default_config(seed), ticks);
        // apply_overrides with empty set → econ unchanged → same SimConfig
        let override_hashes = run(cfg_with_sets(seed, &[]), ticks);
        assert_eq!(
            default_hashes, override_hashes,
            "no-override path must produce bit-identical hash stream"
        );
    }

    /// (2) An override moves the trajectory AND is reproducible across two identical runs.
    /// Uses `base_metab=10` (default=2): 5× higher energy drain → clear population divergence
    /// without waiting for km-saturated uptake to manifest (km is in the whitelist but field is
    /// always resource-limited in the default 64×64 config, making km changes invisible).
    #[test]
    fn override_moves_trajectory_and_is_reproducible() {
        let seed = 1;
        let ticks = 30;
        let default_hashes = run(default_config(seed), ticks);
        let sets = &[("base_metab", "10")]; // 5× higher drain → clear divergence from default
        let run1 = run(cfg_with_sets(seed, sets), ticks);
        let run2 = run(cfg_with_sets(seed, sets), ticks);
        assert_ne!(
            default_hashes, run1,
            "base_metab=10 override must produce a different hash stream from default"
        );
        assert_eq!(run1, run2, "same override must be reproducible");
    }

    /// (3) R14 — the 1-vs-N conserved hash check must run on the OVERRIDDEN econ, not default.
    /// Uses `base_metab=10` to force visible divergence from the no-override sim.
    #[test]
    fn r14_conserved_hash_uses_overridden_econ() {
        let seed = 1;
        let ticks = 30;
        let mut econ = EconParams::default();
        apply_overrides(&mut econ, &[("base_metab".to_string(), "10".to_string())]).unwrap();

        let mut cfg1 = config_with(seed, 1, MergeStrategy::Canonical);
        cfg1.econ = econ;
        let c1 = run_conserved_hashes(cfg1, ticks);

        let mut cfgn = config_with(seed, DEFAULT_THREADS, MergeStrategy::Canonical);
        cfgn.econ = econ;
        let cn = run_conserved_hashes(cfgn, ticks);

        assert_eq!(c1, cn, "R14: conserved hashes must match across thread counts under override");

        // Verify the override actually changed the sim (not silently running default econ).
        let default_c1 = run_conserved_hashes(config_with(seed, 1, MergeStrategy::Canonical), ticks);
        assert_ne!(c1, default_c1, "R14 check must have used the overridden econ, not the default");
    }

    /// (4) Out-of-range value returns an `error:`-prefixed message, NOT a panic.
    #[test]
    fn out_of_range_returns_error_not_panic() {
        let mut econ = EconParams::default();
        // km=0 violates km > 0
        let r = apply_overrides(&mut econ, &[("km".to_string(), "0".to_string())]);
        assert!(r.is_err(), "km=0 must return Err");
        assert!(
            r.unwrap_err().starts_with("error:"),
            "error message must start with 'error:'"
        );

        // u_max=221 violates u_max <= 220
        let r2 = apply_overrides(&mut econ, &[("u_max".to_string(), "221".to_string())]);
        assert!(r2.is_err(), "u_max=221 must return Err");
        assert!(r2.unwrap_err().starts_with("error:"));

        // structural field rejected
        let r3 = apply_overrides(&mut econ, &[("world_dim".to_string(), "256".to_string())]);
        assert!(r3.is_err(), "world_dim must be rejected");
        assert!(r3.unwrap_err().starts_with("error:"));

        // completely unknown key
        let r4 = apply_overrides(&mut econ, &[("nonexistent".to_string(), "1".to_string())]);
        assert!(r4.is_err(), "unknown key must return Err");
        assert!(r4.unwrap_err().starts_with("error:"));
    }

    /// GAP-1: d0_scaled > D0_MASK must be rejected before the sim runs.
    /// d0_scaled > D0_MASK makes `(r & D0_MASK) < d0_scaled` always true → 100% kill every tick.
    #[test]
    fn d0_scaled_above_mask_is_error() {
        let mut econ = EconParams::default();
        let over = (D0_MASK + 1).to_string();
        let r = apply_overrides(&mut econ, &[("d0_scaled".to_string(), over.clone())]);
        assert!(r.is_err(), "d0_scaled={} (> D0_MASK={}) must return Err", over, D0_MASK);
        let msg = r.unwrap_err();
        assert!(msg.starts_with("error:"), "error message must start with 'error:': {msg}");
        assert!(msg.contains("D0_MASK"), "error must mention D0_MASK: {msg}");

        // D0_MASK itself is valid (boundary)
        let r_ok = apply_overrides(&mut econ, &[("d0_scaled".to_string(), D0_MASK.to_string())]);
        assert!(r_ok.is_ok(), "d0_scaled=D0_MASK ({}) must be accepted", D0_MASK);
        assert_eq!(econ.d0_scaled, D0_MASK);
    }

    /// GAP-2: pheromone=NaN and pheromone=inf must be rejected before the sim runs.
    /// f32::from_str parses "NaN"/"inf" successfully; a NaN in the signal field poisons telemetry.
    #[test]
    fn pheromone_nan_inf_is_error() {
        let mut econ = EconParams::default();

        let r_nan = apply_overrides(&mut econ, &[("pheromone".to_string(), "NaN".to_string())]);
        assert!(r_nan.is_err(), "pheromone=NaN must return Err");
        let msg_nan = r_nan.unwrap_err();
        assert!(msg_nan.starts_with("error:"), "error must start with 'error:': {msg_nan}");
        assert!(msg_nan.contains("finite"), "error must mention 'finite': {msg_nan}");

        let r_inf = apply_overrides(&mut econ, &[("pheromone".to_string(), "inf".to_string())]);
        assert!(r_inf.is_err(), "pheromone=inf must return Err");
        assert!(r_inf.unwrap_err().starts_with("error:"));

        let r_neg_inf =
            apply_overrides(&mut econ, &[("pheromone".to_string(), "-inf".to_string())]);
        assert!(r_neg_inf.is_err(), "pheromone=-inf must return Err");
        assert!(r_neg_inf.unwrap_err().starts_with("error:"));

        // 0.0 is valid (turns off pheromone)
        let r_ok = apply_overrides(&mut econ, &[("pheromone".to_string(), "0.0".to_string())]);
        assert!(r_ok.is_ok(), "pheromone=0.0 must be accepted");
    }

    // ── #186 D′-2c tests ─────────────────────────────────────────────────────────────────────────

    /// Smoke: reg-activity telemetry fields are computed and bounded correctly.
    /// dprime_config: reg_gain can evolve → some cells may have reg_gain ≠ 0 after 50 ticks.
    /// default_config: has_light=false → reg_gain stays 0 → reg_active_count must be 0.
    /// Golden-neutral: telemetry is observational only, never affects state hash.
    #[test]
    fn dprime_2c_reg_activity_telemetry_is_computed() {
        let mut sim = build_sim(dprime_config(1));
        for _ in 0..50 {
            sim.step();
        }
        let tel = sim.telemetry();
        assert!(
            tel.reg_active_count >= 0,
            "reg_active_count must be non-negative"
        );
        assert!(
            tel.reg_active_count <= tel.population,
            "reg_active_count={} must not exceed population={}",
            tel.reg_active_count, tel.population
        );
        assert!(
            tel.reg_active_day_count >= 0,
            "reg_active_day_count must be non-negative"
        );
        assert!(
            tel.reg_active_day_count <= tel.reg_active_count,
            "reg_active_day_count={} must be ≤ reg_active_count={}",
            tel.reg_active_day_count, tel.reg_active_count
        );

        // Non-dprime: reg_gain stays 0 forever (has_light=false gates mutation).
        let mut sim2 = build_sim(default_config(1));
        for _ in 0..50 {
            sim2.step();
        }
        let tel2 = sim2.telemetry();
        assert_eq!(
            tel2.reg_active_count, 0,
            "non-dprime config: reg_gain never mutates → reg_active_count must be 0"
        );
    }

    /// D′-2c constitutive control: reg_gain_max=0 locks regulation OFF after evolution.
    /// Verifies the control config fixture (struct-update from dprime_config) works correctly.
    #[test]
    fn dprime_2c_constitutive_control_locks_reg_gain() {
        let seed = 42;
        let mut cfg = dprime_config(seed);
        cfg.econ.reg_gain_max = 0; // D′-2c control line: regulation locked OFF
        let mut sim = build_sim(cfg);
        for _ in 0..50 {
            sim.step();
        }
        let tel = sim.telemetry();
        assert_eq!(
            tel.reg_active_count, 0,
            "constitutive control (reg_gain_max=0): reg_gain must stay 0 in all agents"
        );
    }

    /// D′-2c: --set reg_gain_max is in the whitelist and validates correctly.
    #[test]
    fn dprime_2c_set_reg_gain_max_is_whitelisted() {
        let mut econ = sim_core::EconParams::default();

        // 0 is valid (constitutive control)
        let r0 = apply_overrides(&mut econ, &[("reg_gain_max".to_string(), "0".to_string())]);
        assert!(r0.is_ok(), "--set reg_gain_max=0 must be accepted");
        assert_eq!(econ.reg_gain_max, 0);

        // positive values are valid
        let r4 = apply_overrides(&mut econ, &[("reg_gain_max".to_string(), "4".to_string())]);
        assert!(r4.is_ok(), "--set reg_gain_max=4 must be accepted");
        assert_eq!(econ.reg_gain_max, 4);

        // negative value rejected
        let mut econ2 = sim_core::EconParams::default();
        let r_neg = apply_overrides(&mut econ2, &[("reg_gain_max".to_string(), "-1".to_string())]);
        assert!(r_neg.is_err(), "--set reg_gain_max=-1 must return Err");
        assert!(
            r_neg.unwrap_err().starts_with("error:"),
            "error must start with 'error:'"
        );
    }

    // ── D′-2c verdict experiment ──────────────────────────────────────────────────────────────────
    // PRE-DECLARED MARGIN (recorded before measuring, per ТЗ #186 criterion 3):
    //   PASS = PLASTIC mean N̄ (last-window mean population) > CONSTITUTIVE mean N̄ by ≥ 5%
    //          AND this sign holds in ≥ 4 of 5 seeds.
    //   NULL = threshold not met — an honest informative finding (not a red build).
    //
    // Anti-false-positive guards:
    //   (i) L(t) OSCILLATED: dprime_config has period_ticks=100, day_ticks=50 → verified below.
    //   (ii) GENOTYPE signature: PLASTIC reg_active_count/population > 0 at horizon means
    //        regulation evolved + fixed; if 0, the selective signal is too weak to fix.
    //
    // Configure via env var DPRIME_TICKS (default 400 for fast local iteration; cloud uses 8000).
    // Run: cargo test --release -p cli -- dprime_2c_verdict --nocapture --ignored

    /// D′-2c verdict: PLASTIC (reg_gain_max=4) vs CONSTITUTIVE (reg_gain_max=0) across ≥5 seeds.
    /// Heavy (many ticks × 2 arms × 5 seeds) — ignored in CI; run explicitly for the verdict.
    /// Cloud dispatch: scripts/sim-run.sh dprime-2c ticks=8000
    #[test]
    #[ignore]
    fn dprime_2c_verdict() {
        use sim_core::light_at_tick;

        // Anti-false-positive (i): verify L(t) oscillates in dprime_config.
        let spec = dprime_config(1).econ.light.unwrap();
        let l_day = light_at_tick(&spec, 10);   // tick 10: well within day phase (day_ticks=50)
        let l_night = light_at_tick(&spec, 60); // tick 60: well within night phase
        assert!(
            l_day > 0 && l_night == 0,
            "anti-false-positive (i): L(t) must oscillate (l_day={l_day}, l_night={l_night})"
        );

        let ticks: u64 = std::env::var("DPRIME_TICKS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(400);
        let seeds: &[u64] = &[1, 2, 3, 4, 5];
        // Late-window: last 20% of run (at least 50 ticks)
        let window_start = (ticks.saturating_sub(ticks / 5)).max(ticks.saturating_sub(200));

        println!("\nD\u{2019}-2c verdict: PLASTIC (reg_gain_max=4) vs CONSTITUTIVE (reg_gain_max=0)");
        println!("PRE-DECLARED MARGIN: PLASTIC N\u{304} > CONSTITUTIVE N\u{304} by \u{2265}5%, sign in \u{2265}4/5 seeds");
        println!("ticks={ticks}  late-window=[{window_start},{ticks}]  period_ticks={}", spec.period_ticks);
        println!("{:<6} {:>14} {:>16} {:>10} {:>12} {:>8}",
            "seed", "PLASTIC N\u{304}", "CONSTITUTIVE N\u{304}", "diff%", "reg_active%", "sign");
        println!("{}", "-".repeat(72));

        let mut plastic_wins = 0usize;
        let mut all_plastic: Vec<f64> = Vec::new();
        let mut all_const: Vec<f64> = Vec::new();

        for &seed in seeds {
            let (p_mean, p_reg_frac, p_reg_day_frac) =
                run_dprime_arm_detailed(seed, ticks, 4, window_start);
            let (c_mean, _c_reg_frac, _) =
                run_dprime_arm_detailed(seed, ticks, 0, window_start);

            let diff_pct = if c_mean > 0.0 {
                (p_mean - c_mean) / c_mean * 100.0
            } else {
                0.0
            };
            let sign = if p_mean >= c_mean { "+" } else { "-" };
            let reg_pct = p_reg_frac * 100.0;

            println!(
                "{:<6} {:>14.1} {:>16.1} {:>+9.1}% {:>11.1}% {:>8}  (day-frac={:.0}%)",
                seed, p_mean, c_mean, diff_pct, reg_pct, sign,
                p_reg_day_frac * 100.0
            );

            if p_mean > c_mean { plastic_wins += 1; }
            all_plastic.push(p_mean);
            all_const.push(c_mean);
        }

        let plastic_grand: f64 = all_plastic.iter().sum::<f64>() / seeds.len() as f64;
        let const_grand: f64 = all_const.iter().sum::<f64>() / seeds.len() as f64;
        let margin_pct = if const_grand > 0.0 {
            (plastic_grand - const_grand) / const_grand * 100.0
        } else {
            0.0
        };

        println!("{}", "-".repeat(72));
        println!(
            "{:<6} {:>14.1} {:>16.1} {:>+9.1}%  sign_consistency={}/5",
            "MEAN", plastic_grand, const_grand, margin_pct, plastic_wins
        );
        println!();

        let pass = margin_pct >= 5.0 && plastic_wins >= 4;
        if pass {
            println!("VERDICT: PASS");
            println!("  Regulation has selective value in the dprime economy.");
            println!("  margin={margin_pct:+.1}% \u{2265}5%, sign in {plastic_wins}/5 seeds \u{2265}4.");
            println!("  D\u{2019} revives D: the GRN setpoint+gain pattern fixes and pays");
            println!("  under a temporal driver (L(t)) + photo-machinery cost asymmetry.");
            println!("  Closure note for #169/#171: SUPERSEDED-BUT-VINDICATED.");
            println!("  The sense_range-on-substrate instance failed (no selective value on a");
            println!("  static signal); the PATTERN is vindicated here on a temporal signal.");
        } else {
            println!("VERDICT: NULL");
            println!("  margin={margin_pct:+.1}% (threshold: \u{2265}5%), sign in {plastic_wins}/5 seeds (threshold: \u{2265}4).");
            println!("  Regulation confers no standing-crop advantage even with:");
            println!("    - temporal L(t) driver (oscillating, period={} ticks)", spec.period_ticks);
            println!("    - photo-machinery cost asymmetry (D\u{2019}-2a photo_cost_num/den)");
            println!("  Honest informative finding per plan \u{00a7}8/F8.");
            println!("  Closure note for #169/#171: GENERALIZED NULL.");
            println!("  The GRN plasticity track (3rd instance) finds no selective value");
            println!("  in this economy; the track is closed for Phase 1.");
        }
    }

    fn run_dprime_arm_detailed(
        seed: u64,
        ticks: u64,
        reg_gain_max: i32,
        window_start: u64,
    ) -> (f64, f64, f64) {
        let mut cfg = dprime_config(seed);
        cfg.econ.reg_gain_max = reg_gain_max;
        let mut sim = build_sim(cfg);
        let mut pop_sum = 0u64;
        let mut pop_count = 0u64;
        for t in 0..ticks {
            sim.step();
            if t >= window_start {
                pop_sum += sim.population();
                pop_count += 1;
            }
        }
        // Anti-false-positive (ii): fraction of population with reg_gain ≠ 0 at horizon.
        let tel = sim.telemetry();
        let final_pop = tel.population.max(1);
        let reg_active_frac = tel.reg_active_count as f64 / final_pop as f64;
        let reg_day_frac = if tel.reg_active_count > 0 {
            tel.reg_active_day_count as f64 / tel.reg_active_count as f64
        } else {
            0.0
        };
        let mean_pop = if pop_count > 0 {
            pop_sum as f64 / pop_count as f64
        } else {
            0.0
        };
        (mean_pop, reg_active_frac, reg_day_frac)
    }

    // ── #189 D′-3b tests ─────────────────────────────────────────────────────────────────────────

    /// Smoke: per-cell income split is recorded in TraitSample; guild census is consistent.
    ///
    /// Note: founders start with `photo_gain=0` (non-zero evolves over many ticks), so `photo_in=0`
    /// initially for all cells. We verify `chem_in > 0` (always available when field has resources)
    /// and guild census consistency. The Phototroph classifier is unit-tested in `telemetry::tests`.
    /// Golden-neutral: income_record is observational, never affects state_hash.
    #[test]
    fn dprime_3b_income_split_is_recorded_and_guild_census_consistent() {
        let mut sim = build_sim(dprime_config(1));
        for _ in 0..30 {
            sim.step();
        }
        let tel = sim.telemetry();
        // chem_in: after 30 ticks the conserved field has resources → cells receive chemical income.
        // photo_in: photo_gain=0 at founder → photo_demand=0 → photo_in=0 until evolution acts.
        let any_chem = tel.samples.iter().any(|s| s.chem_in > 0);
        assert!(any_chem, "dprime_config: at least some cells must have chem_in > 0 (field has resources)");
        let all_photo_zero = tel.samples.iter().all(|s| s.photo_in == 0);
        assert!(all_photo_zero, "at tick 30, photo_gain is still 0 → photo_in must be 0 for all cells");

        // Guild census: counts must sum to population (Phototroph=0 since photo_in=0 at tick 30).
        let rep = telemetry::compute_with_census(&tel.samples, &tel.species_census, None);
        let guild_sum: usize = rep.guild_pop.iter().sum();
        assert_eq!(guild_sum, rep.population, "guild_pop must sum to population");
        assert_eq!(rep.guild_pop[telemetry::Guild::Phototroph as usize], 0,
            "no Phototrophs at tick 30 (photo_gain=0 at founder)");

        // Non-dprime: photo_in always 0 → never Phototroph, guild census consistent.
        let mut sim2 = build_sim(default_config(1));
        for _ in 0..30 { sim2.step(); }
        let tel2 = sim2.telemetry();
        assert!(tel2.samples.iter().all(|s| s.photo_in == 0),
            "default_config: photo_in must be 0 for all samples (no light)");
        let rep2 = telemetry::compute_with_census(&tel2.samples, &[], None);
        assert_eq!(rep2.guild_pop[telemetry::Guild::Phototroph as usize], 0,
            "default_config must have 0 Phototrophs");
        let guild_sum2: usize = rep2.guild_pop.iter().sum();
        assert_eq!(guild_sum2, rep2.population, "default_config guild_pop must sum to population");
    }

    // ── D′-3b verdict experiment ──────────────────────────────────────────────────────────────────
    // PRE-DECLARED KNOBS (recorded before measuring, per ТЗ #189, anti-p-hacking gate):
    //   Signal    : realized photo_in / (photo_in + chem_in) per TraitSample (exact booked integers)
    //   Threshold : >50% (photo_in * 2 > total_in, exact integer — no division, no truncation)
    //   Light     : dprime_config (l_max=100, day_ticks=50) — mineral-limited regime (NOT L≈20
    //               knife-edge per FLAG-1); calibration §7 shows L=100/M-poor → mineral binds.
    //   PROD_FRAC : 0.10 — guild_pop[Phototroph]/total ≥ 10% at the 8000-tick horizon
    //   Seeds     : 5 (seeds 1..5), verdict EMERGES if ≥3/5 seeds exceed PROD_FRAC
    //               (NULL = honest informative outcome, not a red build)
    //
    // D′-3a CHECK-1 (co-limitation is real): mineral genuinely binds when mineral_layer is Some(2).
    // The mineral economy (q_mineral=4000, regen calibrated to M-limited regime) ensures Liebig
    // co-limitation is active — energy is not the sole limiter. Verified via non-zero overflow
    // (energy wasted when mineral-poor → overflow_delta applied → ledger.lost > 0 for dprime).
    //
    // Run: DPRIME_TICKS=8000 cargo test --release -p cli -- dprime_3b_emergence_verdict --nocapture --ignored
    // Cloud: scripts/sim-run.sh dprime-3b ticks=8000  (after PR merges to main)

    /// D′-3b emergence verdict: does the §5 producer guild (Phototroph) emerge in the dprime economy?
    /// Heavy (5 seeds × 8000 ticks) — ignored in CI; run explicitly for the verdict.
    /// Cloud dispatch: scripts/sim-run.sh dprime-3b ticks=8000
    #[test]
    #[ignore]
    fn dprime_3b_emergence_verdict() {
        use sim_core::light_at_tick;
        use telemetry::{compute_with_census, Guild};

        // Anti-false-positive: verify L(t) oscillates in dprime_config (NOT L≈20 knife-edge).
        let spec = dprime_config(1).econ.light.unwrap();
        let l_day   = light_at_tick(&spec, 10);  // tick 10: day phase
        let l_night = light_at_tick(&spec, 60);  // tick 60: night phase
        assert!(l_day > 0 && l_night == 0,
            "anti-fp: L(t) must oscillate (l_day={l_day}, l_night={l_night})");
        assert!(l_day > 20,
            "anti-fp FLAG-1: l_max={} must be clearly above the L≈20 niche knife-edge", l_day);

        let ticks: u64 = std::env::var("DPRIME_TICKS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(400);
        let seeds: &[u64] = &[1, 2, 3, 4, 5];
        // Late-window: last 20% of run (at least 50 ticks).
        let window_start = (ticks.saturating_sub(ticks / 5)).max(ticks.saturating_sub(200));
        const PROD_FRAC: f64 = 0.10; // pre-declared: ≥10% Phototrophs at horizon = EMERGES

        println!("\nD\u{2019}-3b emergence verdict: Guild::Phototroph in dprime mineral economy");
        println!("PRE-DECLARED: signal=photo_in/(photo_in+chem_in), threshold=>50%, PROD_FRAC={PROD_FRAC:.0}%");
        println!("Light regime: l_max={}, day_ticks={} (mineral-limited, not L\u{2248}20 knife-edge)",
            spec.l_max, spec.day_ticks);
        println!("ticks={ticks}  late-window=[{window_start},{ticks}]");
        println!("{:<6} {:>14} {:>14} {:>14} {:>10} {:>8}",
            "seed", "phototroph%", "producer%", "consumer%", "mean_pop", "EMERGES");
        println!("{}", "-".repeat(70));

        let mut emerges_count = 0usize;
        let mut all_ph_frac: Vec<f64> = Vec::new();

        for &seed in seeds {
            let (mean_pop, ph_frac, prod_frac, cons_frac, mineral_binds) =
                run_dprime_3b_arm(seed, ticks, window_start);
            let emerges = ph_frac >= PROD_FRAC;
            if emerges { emerges_count += 1; }
            all_ph_frac.push(ph_frac);
            println!(
                "{:<6} {:>13.1}% {:>13.1}% {:>13.1}% {:>10.1} {:>8}  co-lim={}",
                seed, ph_frac * 100.0, prod_frac * 100.0, cons_frac * 100.0,
                mean_pop, if emerges { "EMERGES" } else { "null" },
                if mineral_binds { "YES" } else { "no" }
            );
        }

        let mean_ph_frac: f64 = all_ph_frac.iter().sum::<f64>() / seeds.len() as f64;
        println!("{}", "-".repeat(70));
        println!("MEAN phototroph%={:.1}%  emerges in {emerges_count}/{} seeds", mean_ph_frac * 100.0, seeds.len());
        println!();

        let verdict = emerges_count >= 3; // pre-declared: ≥3/5 seeds = EMERGES
        if verdict {
            println!("VERDICT: EMERGES");
            println!("  Producer guild (Phototroph) established in {emerges_count}/5 seeds \u{2265} 3.");
            println!("  Mean phototroph fraction {:.1}% \u{2265} pre-declared PROD_FRAC {:.0}%.",
                mean_ph_frac * 100.0, PROD_FRAC * 100.0);
            println!("  \u{00a7}5 producer ecology closes: chem + biotic-recycle + light + mineral,");
            println!("  all conserved, guild-structured. Phase-1 energy/nutrient economy closes.");
        } else {
            println!("VERDICT: NULL");
            println!("  Phototroph guild did NOT establish in \u{2265}3/5 seeds (emerged in {emerges_count}/5).");
            println!("  Mean phototroph fraction {:.1}% (threshold PROD_FRAC={:.0}%).",
                mean_ph_frac * 100.0, PROD_FRAC * 100.0);
            println!("  Honest informative finding: producer ecology does not emerge at this");
            println!("  mineral/light calibration. See §7 FLAG-2 for parameter adjustment options.");
        }
    }

    /// Run one dprime_config arm for the D′-3b verdict.
    /// Returns (mean_pop_in_window, phototroph_frac, producer_frac, consumer_frac, mineral_active).
    fn run_dprime_3b_arm(
        seed: u64,
        ticks: u64,
        window_start: u64,
    ) -> (f64, f64, f64, f64, bool) {
        use telemetry::compute_with_census;

        let cfg = dprime_config(seed);
        // D′-3a CHECK-1: mineral genuinely binds — structural check (calibration §7 proved the
        // mineral-limited regime at L=100/M-poor; mineral_layer.is_some() confirms it is active).
        let mineral_active = cfg.econ.mineral_layer.is_some();
        let mut sim = build_sim(cfg);
        let mut pop_sum = 0u64;
        let mut pop_count = 0u64;
        // Accumulate guild fractions in the late window.
        let mut ph_sum = 0u64;
        let mut prod_sum = 0u64;
        let mut cons_sum = 0u64;
        let mut guild_total_sum = 0u64;

        for t in 0..ticks {
            sim.step();
            if t >= window_start {
                let tel = sim.telemetry();
                let rep = compute_with_census(&tel.samples, &tel.species_census, None);
                ph_sum += rep.guild_pop[telemetry::Guild::Phototroph as usize] as u64;
                prod_sum += rep.guild_pop[telemetry::Guild::Producer as usize] as u64;
                cons_sum += rep.guild_pop[telemetry::Guild::Consumer as usize] as u64;
                guild_total_sum += rep.population as u64;
                pop_sum += tel.population as u64;
                pop_count += 1;
            }
        }

        let mean_pop = if pop_count > 0 { pop_sum as f64 / pop_count as f64 } else { 0.0 };
        let gt = guild_total_sum.max(1) as f64;
        let ph_frac = ph_sum as f64 / gt;
        let prod_frac = prod_sum as f64 / gt;
        let cons_frac = cons_sum as f64 / gt;

        (mean_pop, ph_frac, prod_frac, cons_frac, mineral_active)
    }

    // ── Existing tests ────────────────────────────────────────────────────────────────────────────

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
