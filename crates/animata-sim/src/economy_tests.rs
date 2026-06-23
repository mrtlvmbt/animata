//! Energy-economy ledger + REBALANCE SANDBOX. Derives, from the tunable economy params + the real sim
//! formulas, every TIMESCALE and RATIO that governs survival & reproduction — so the (otherwise
//! emergent, invisible) economy is legible and the rebalance can be dialled in ON PAPER before any
//! plan/spike. `economy_report` prints the CURRENT (config) regime; `economy_candidates` evaluates
//! alternative param sets against the HEALTHY target band.
//!
//! The pathology (2026-06 local-carrying-capacity spike): a fed cell's COAST time (buffer ÷ burn) ≫
//! its LIFESPAN, and photosynthetic income ≫ burn — so STARVATION is a dead mortality channel; density
//! (shading) only throttles BIRTHS, never kills ⇒ overshoot persists ~a lifespan ⇒ oscillation.
//! Biology (web-research): density regulation needs density-dependent MORTALITY, not just lower
//! fecundity; r-strategist unicells have SHORT lives + low stress survival (the opposite of here).

use crate::config::*;

/// The tunable economy params (defaults = current config consts). Vary these to find a healthy set.
#[derive(Clone, Copy)]
struct Econ {
    metab: f32,      // SIM_BASE_METABOLISM
    max_e: f32,      // MAX_ENERGY (buffer ceiling, before storage organ)
    start: f32,      // START_ENERGY
    repro: f32,      // REPRO_ENERGY
    photo_rate: f32, // PHOTO_RATE
    lifespan: f32,   // LIFESPAN (ticks)
}

impl Econ {
    fn current() -> Self {
        Econ {
            metab: SIM_BASE_METABOLISM,
            max_e: MAX_ENERGY,
            start: START_ENERGY,
            repro: REPRO_ENERGY,
            photo_rate: PHOTO_RATE,
            lifespan: LIFESPAN,
        }
    }

    /// Derived health metrics for a UNICELL autotroph (n=1, photo=1) — the founder/base case.
    /// burn (sim.rs): metab·biomass^0.75·TICK_LEN. income (autotrophy.rs): photo_rate·photo·light·shading·TICK_LEN.
    fn metrics(&self) -> Metrics {
        let burn = self.metab * 1.0 * TICK_LEN; // biomass^0.75 = 1
        let income = self.photo_rate * 1.0 * 1.0 * 1.0 * TICK_LEN; // light=1, shading=1
        let net = income - burn;
        let coast = self.max_e / burn; // ticks to starve from full, zero income
        let to_repro = if net > 0.0 { (self.repro - self.start) / net } else { f32::INFINITY };
        let broods_per_life = self.lifespan / to_repro; // ~ how many times it reproduces in a lifetime
        let shade_break = burn / income; // shading at which income==burn (cell loses energy below)
        let n_break_sc = |sc: f32| sc * (1.0 / shade_break - 1.0); // autotrophs/cell to zero net
        Metrics {
            burn,
            income,
            ratio: income / burn,
            coast_lives: coast / self.lifespan,
            to_repro,
            broods_per_life,
            n_break_30: n_break_sc(30.0),
        }
    }
}

struct Metrics {
    burn: f32,
    income: f32,
    ratio: f32,
    coast_lives: f32,
    to_repro: f32,
    broods_per_life: f32,
    n_break_30: f32,
}

impl Metrics {
    /// HEALTHY target band (from the ledger + biology): starvation reachable within a life, density
    /// throttle bites at sane per-cell counts, a creature reproduces a handful of times per life.
    fn health(&self) -> &'static str {
        let ok_coast = self.coast_lives < 0.5; // starved cell dies well within a lifespan
        let ok_ratio = (2.0..=12.0).contains(&self.ratio); // density can reach net-zero realistically
        let ok_broods = (3.0..=30.0).contains(&self.broods_per_life); // turnover, not immortal-fecund
        match (ok_coast, ok_ratio, ok_broods) {
            (true, true, true) => "✓ HEALTHY",
            _ => "✗ (coast/ratio/broods)",
        }
    }

    fn line(&self, label: &str) {
        println!(
            "  {label:<22} burn {:>7.3} income {:>7.3} ratio {:>5.1}× coast {:>6.2} lives  \
             →repro {:>5.0}t broods/life {:>5.1}  n@net0(sc30) {:>6.0}  {}",
            self.burn, self.income, self.ratio, self.coast_lives, self.to_repro,
            self.broods_per_life, self.n_break_30, self.health(),
        );
    }
}

#[test]
fn economy_report() {
    println!("\n========== ENERGY-ECONOMY LEDGER (current config) ==========");
    println!("metab={SIM_BASE_METABOLISM} max={MAX_ENERGY} start={START_ENERGY} repro={REPRO_ENERGY} \
              photo_rate={PHOTO_RATE} lifespan={LIFESPAN} tick_len={TICK_LEN}\n");
    Econ::current().metrics().line("CURRENT");

    // Predation income (sim.rs): (prey_bm·CELL_BIOMASS_COST + prey_e)·MEAT_EFFICIENCY·carnivory.
    let predation = (1.0 * CELL_BIOMASS_COST + MAX_ENERGY) * MEAT_EFFICIENCY;
    println!("\n  predation: one kill (unicell full tank) = {:.1} e (= {:.1}× repro)  meat_eff {MEAT_EFFICIENCY} aerobic {AEROBIC_GAIN}",
        predation, predation / REPRO_ENERGY);
    println!("  forage floor (water, global F7 crutch): {:.4} e/tick/head @1000 heads", WATER_CAPACITY / 1000.0 * TICK_LEN);
    println!("============================================================\n");

    // Regime markers (current = pathological; re-pin to healthy on rebalance).
    let m = Econ::current().metrics();
    assert!(m.coast_lives > 10.0, "current: coast {:.1} lives (≫1 ⇒ dead starvation channel)", m.coast_lives);
    assert!(m.ratio > 30.0, "current: income/burn {:.0}× (≫ ⇒ density barely bites)", m.ratio);
}

#[test]
fn economy_candidates() {
    println!("\n========== REBALANCE CANDIDATES (target: coast<0.5 lives, ratio 2–12×, broods 3–30) ==========");
    let cur = Econ::current();
    cur.metrics().line("A current");

    // B — raise metabolism ×30 only (burn up, income unchanged ⇒ ratio falls, coast shortens).
    let mut b = cur;
    b.metab = SIM_BASE_METABOLISM * 30.0;
    b.metrics().line("B metab×30");

    // C — metab ×30 + photo ×6 (restore a healthy net while keeping a short coast).
    let mut c = cur;
    c.metab = SIM_BASE_METABOLISM * 30.0;
    c.photo_rate = PHOTO_RATE * 6.0;
    c.metrics().line("C metab×30 photo×6");

    // D — metab ×100 + photo ×20 (full rescale of the energy clock relative to lifespan).
    let mut d = cur;
    d.metab = SIM_BASE_METABOLISM * 100.0;
    d.photo_rate = PHOTO_RATE * 20.0;
    d.metrics().line("D metab×100 photo×20");

    // E — shrink the BUFFER instead: max/start/repro ÷10, metab ×10 (coast = max/burn ÷100).
    let mut e = cur;
    e.max_e = MAX_ENERGY / 10.0;
    e.start = START_ENERGY / 10.0;
    e.repro = REPRO_ENERGY / 10.0;
    e.metab = SIM_BASE_METABOLISM * 10.0;
    e.metrics().line("E buffer÷10 metab×10");

    // F — gentle: metab ×20, photo ×4, repro ×1.5 (slower broods).
    let mut f = cur;
    f.metab = SIM_BASE_METABOLISM * 20.0;
    f.photo_rate = PHOTO_RATE * 4.0;
    f.repro = REPRO_ENERGY * 1.5;
    f.metrics().line("F metab×20 photo×4 repro×1.5");

    // G — DESIGNED to hit all three bands (keeps the energy currency: predation/forage/storage scale
    // unchanged; only metab, photo_rate, repro move). burn 0.444 ⇒ coast 0.30 lives; ratio 5×; the high
    // repro threshold (267 over START) slows broods to ~10/life (low intrinsic r ⇒ damped logistic).
    let mut g = cur;
    g.metab = SIM_BASE_METABOLISM * 88.8; // → burn 0.444/tick
    g.photo_rate = PHOTO_RATE * 7.4; // → income 2.22/tick (ratio 5×)
    g.repro = 317.0; // 267 over START=50 ⇒ ~150t/brood ⇒ ~10 broods/life
    g.metrics().line("G designed");

    // H — DESIGNED via buffer-shrink (keeps metab/photo modest, rescales the energy CURRENCY ÷~56 —
    // so START/REPRO/MAX/predation/forage/storage/cell-cost ALL shrink together). Same health, more
    // coupled consts to move. Shown for comparison.
    let mut h = cur;
    h.max_e = 3.6;
    h.start = 0.56;
    h.repro = 3.56;
    h.photo_rate = 0.25;
    h.metrics().line("H buffer-shrink");
    println!("  (G keeps the currency → fewer coupled consts; H rescales everything → messier)");
    println!("================================================================================================\n");
}

/// DIFFERENTIAL metabolism by type (the realistic model — user-directed). Autotrophs SIT: low turnover,
/// brake = LIGHT throttling reproduction (coast may be long, that's fine — a shaded plant doesn't die
/// fast). Heterotrophs are ACTIVE: high metabolism ⇒ short coast ⇒ FOOD-limited, starve fast = the
/// responsive density brake. Web-research: per-N mass-specific rates are closer than raw mass suggests,
/// but the autotroph-sits / heterotroph-burns split is the right ecological model. Base minimum metab ≈ 1.
#[test]
fn economy_differential() {
    println!("\n========== DIFFERENTIAL METABOLISM (autotroph low / heterotroph high, base_min≈1) ==========");
    let base_min = 1.0f32; // user: "базовый минимальный метаболизм 1"
    let hetero_activity = 8.0f32; // animal tissue (muscle/nerve) ~8× the sitting-autotroph baseline

    // Autotroph: pays base_min only, income from photosynthesis. Brake = light→repro (NOT starvation).
    let burn_a = base_min * 1.0 * TICK_LEN; // 0.1/tick
    let photo_rate = 5.0f32; // modest (×1.67 of current 3.0)
    let income_a = photo_rate * 1.0 * TICK_LEN; // 0.5/tick
    let net_a = income_a - burn_a;
    let coast_a = MAX_ENERGY / burn_a;
    let to_repro_a = (REPRO_ENERGY - START_ENERGY) / net_a;
    let shade_break_a = burn_a / income_a;
    let n_break_a = 30.0 * (1.0 / shade_break_a - 1.0);
    println!("  AUTOTROPH (metab {base_min}, photo_rate {photo_rate}):");
    println!("    burn {burn_a:.3} income {income_a:.3} ratio {:.1}× net {net_a:.3}", income_a / burn_a);
    println!("    coast {:.2} lives (LONG ok — sitter)  →repro {to_repro_a:.0}t  broods/life {:.1}", coast_a / LIFESPAN, LIFESPAN / to_repro_a);
    println!("    shading@net0 {shade_break_a:.3}  → cell caps at {n_break_a:.0} autotrophs (softcap30)  ← light brake on REPRO");

    // Heterotroph: pays base_min × activity, income from predation. Brake = FOOD→fast starvation.
    let burn_h = base_min * hetero_activity * 1.0 * TICK_LEN; // 0.8/tick (unicell; ×kleiber for bigger)
    let coast_h = MAX_ENERGY / burn_h;
    let one_kill = (1.0 * CELL_BIOMASS_COST + MAX_ENERGY) * MEAT_EFFICIENCY; // 72.8e
    let kill_sustains = one_kill / burn_h; // ticks one kill buys
    println!("  HETEROTROPH (metab {base_min}×{hetero_activity}={}, predation income):", base_min * hetero_activity);
    println!("    burn {burn_h:.3}  coast {:.2} lives (SHORT ✓ — food-limited, fast starvation = the active brake)", coast_h / LIFESPAN);
    println!("    one kill {one_kill:.1}e sustains {kill_sustains:.0} ticks ⇒ must eat every ~{kill_sustains:.0}t or starve");

    let auto_ok = (2.0..=12.0).contains(&(income_a / burn_a)) && (3.0..=30.0).contains(&(LIFESPAN / to_repro_a));
    let het_ok = coast_h / LIFESPAN < 0.5;
    println!("  → autotroph {} (light-limited repro) ; heterotroph {} (food-limited death)",
        if auto_ok { "✓" } else { "✗" }, if het_ok { "✓" } else { "✗" });
    println!("  KEY: differential metab needs a metabolism-formula change (base + per-cell-type cost),");
    println!("       NOT a uniform SIM_BASE_METABOLISM raise — that is a MECHANISM change for the plan.");
    println!("=============================================================================================\n");
    assert!(auto_ok && het_ok, "differential model should be healthy for both types");
}

/// THE BASE UNIT — economy anchored on a single fundamental, the way physics anchors on `c`.
/// **M₀ = the energy to maintain ONE autotroph cell for ONE tick** (the cheapest tissue = the unit;
/// the user's "базовый минимальный метаболизм 1"). EVERY energy quantity is a multiple of M₀; the only
/// other anchor is the demographic clock LIFESPAN (ticks). The whole economy is then a handful of
/// DIMENSIONLESS design ratios — pick those from biology/stability and ALL the sim consts fall out.
#[test]
fn economy_from_base_unit() {
    // ---- THE ANCHOR ----
    let m0 = 1.0f64; // energy / autotroph-cell / tick. DEFINES the energy unit. (the "speed of light")
    let lifespan = 1500.0f64; // ticks. DEFINES the demographic time scale.

    // ---- DIMENSIONLESS DESIGN VECTOR (the only free choices; everything else is derived) ----
    let a_het = 8.0; // heterotroph metabolic activity: animal tissue burns ~8× a sitting autotroph cell
    let rho = 5.0; // autotroph income/burn ratio: net-positive yet density(shading)-throttleable
    let coast_het_lives = 0.25; // a starved heterotroph dies in ¼ of a lifespan ⇒ FOOD is the active brake
    let broods = 10.0; // reproductions per autotroph lifetime ⇒ LOW intrinsic r ⇒ damped (non-chaotic) logistic
    let start_frac = 0.25; // a newborn starts at ¼ of the energy buffer

    // ---- DERIVE the economy (all in M₀ units / ticks) ----
    let burn_auto = m0; // per tick, unicell autotroph
    let burn_het = a_het * m0; // per tick, unicell heterotroph (active tissue)
    let income_auto = rho * m0; // photosynthesis, full light/no shading
    let net_auto = income_auto - burn_auto; // (rho-1)·M₀
    // buffer sized so the HETEROTROPH coasts `coast_het_lives` of a life ⇒ autotroph (¹⁄₈ burn) coasts 8× longer (sits).
    let buffer = coast_het_lives * lifespan * a_het; // = MAX_ENERGY / M₀
    let to_repro = lifespan / broods; // ticks from START to REPRO at lone-autotroph net
    let repro_minus_start = net_auto * to_repro; // energy gap birth→split
    let start = start_frac * buffer;
    let repro = start + repro_minus_start;
    // predation: size one kill so a heterotroph kill sustains ~one inter-brood interval of its burn.
    let kill_yield = burn_het * to_repro; // energy per kill (in M₀)

    println!("\n================= ECONOMY FROM ONE BASE UNIT (M₀ = autotroph cell·tick) =================");
    println!("ANCHORS:  M₀ = {m0} (energy/auto-cell/tick)   LIFESPAN = {lifespan} ticks");
    println!("DESIGN (dimensionless): a_het={a_het} rho={rho} coast_het={coast_het_lives}life broods={broods} start_frac={start_frac}");
    println!("─ derived economy (× M₀) ─");
    println!("  burn  autotroph {burn_auto:>7.2}/t   heterotroph {burn_het:>7.2}/t   (×{a_het})");
    println!("  income autotroph {income_auto:>6.2}/t   net {net_auto:.2}/t   ratio {rho}×");
    println!("  buffer MAX_ENERGY {buffer:>7.0}    → autotroph coast {:.2} lives / heterotroph coast {:.2} lives",
        buffer / burn_auto / lifespan, buffer / burn_het / lifespan);
    println!("  START {start:.0}  REPRO {repro:.0}  (gap {repro_minus_start:.0}) → {to_repro:.0} t/brood, {broods} broods/life");
    println!("  one kill yields {kill_yield:.0}  (sustains a heterotroph {:.0} t)", kill_yield / burn_het);
    println!("─ MAPPED to sim consts (TICK_LEN={TICK_LEN}; M₀ = SIM_BASE_METABOLISM·TICK_LEN) ─");
    println!("  SIM_BASE_METABOLISM = M₀/TICK_LEN      = {:.1}   (autotroph baseline; was {SIM_BASE_METABOLISM})", m0 / TICK_LEN as f64);
    println!("  heterotroph metab   = a_het·M₀/TICK_LEN = {:.1}   ← needs a per-type metab term (MECHANISM)", a_het * m0 / TICK_LEN as f64);
    println!("  PHOTO_RATE          = income/(photo·TICK_LEN) = {:.1}   (was {PHOTO_RATE})", income_auto / TICK_LEN as f64);
    println!("  MAX_ENERGY {buffer:.0}  START {start:.0}  REPRO {repro:.0}   (was {MAX_ENERGY}/{START_ENERGY}/{REPRO_ENERGY})");
    println!("=========================================================================================\n");

    // sanity: the derived regime sits in the healthy bands by construction.
    assert!((2.0..=12.0).contains(&rho));
    assert!(buffer / burn_het / lifespan < 0.5, "heterotroph must starve within ½ a life");
    assert!((3.0..=30.0).contains(&broods));
}
