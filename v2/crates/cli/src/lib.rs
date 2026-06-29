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
            _ => {
                return Err(format!(
                    "error: --set {key}=…: not an overridable calibration knob. \
                     Valid keys: km, u_max, base_metab, c_div, e_cell, k_size_metab, \
                     k_move_cost, k_sense_cost, excrete, recycle_num, speciation_threshold, \
                     brain_period, metab_period, d0_scaled, pheromone."
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
