//! All tunable constants for the simulation, in one place.

/// Live-tunable simulation parameters (driven by the in-app sliders). Everything
/// here starts from the constants below and can be changed at runtime.
#[derive(Clone, Copy)]
pub struct Params {
    pub food_per_step: f32,
    pub predator_gain: f32,
    pub mutation_rate: f64,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            food_per_step: FOOD_PER_STEP,
            predator_gain: PREDATOR_GAIN,
            mutation_rate: MUTATION_RATE,
        }
    }
}

// ---- World ----
// Giant map: 64× the on-screen viewport area (8× each side). The camera defaults
// to a simplified whole-world overview (creatures render as LOD dots) and
// zooms/pans in to per-creature detail. Food/founder counts scale with area so
// per-biome density matches the old world; only the canvas got bigger.
pub const WORLD_W: f32 = 8800.0;
pub const WORLD_H: f32 = 6080.0;
/// On-screen world viewport (window) size, decoupled from world size.
pub const VIEW_W: f32 = 1100.0;
pub const VIEW_H: f32 = 760.0;

pub const START_CREATURES: usize = 1920;
/// Predators present at start (they hunt herbivores instead of pellets).
pub const START_PREDATORS: usize = 192;
/// Food counts scale with world area so per-biome density is unchanged across the
/// ×16-larger map.
pub const START_FOOD: usize = 25600;
/// Max food pellets present in the world; spawner tops up toward this.
pub const FOOD_CAP: usize = 44800;
/// New food pellets attempted per simulation step.
pub const FOOD_PER_STEP: f32 = 140.8;
/// Spatial-grid cell size for food lookups. Must exceed the largest eating
/// reach (creature radius + food radius) so the 3x3 query stays correct.
pub const GRID_CELL: f32 = 64.0;

// ---- Environment (seasons + droughts) ----
/// Steps per seasonal cycle; food spawn waxes and wanes over this period.
pub const SEASON_PERIOD: f32 = 3000.0;
/// Seasonal food multiplier = SEASON_BASE + SEASON_AMP * sin(2π·tick/period).
pub const SEASON_BASE: f32 = 0.85;
pub const SEASON_AMP: f32 = 0.22;
/// Per-step probability of a drought starting (only while none is active).
pub const DROUGHT_CHANCE: f64 = 0.0006;
/// Drought duration in steps (randomized up to ×2).
pub const DROUGHT_LEN: u64 = 300;
/// Food spawn multiplier during a drought.
pub const DROUGHT_FOOD_MULT: f32 = 0.45;

// ---- Biomes ----
/// Value-noise lattice spacing (px). Bigger -> larger, smoother biome regions.
pub const BIOME_LATTICE: f32 = 220.0;

// ---- Terrain barriers (rivers) ----
// Impassable-ish water carves the world into regions: a creature can only cross
// slowly and at high metabolic cost, so gene flow between sides is throttled and
// clades diverge in near-isolation (allopatric speciation).
/// Lattice spacing for the river noise (large -> few, broad, sweeping rivers).
pub const BARRIER_LATTICE: f32 = 560.0;
/// Half-width (in noise units) of the contour band classed as water. The band
/// |noise - 0.5| < this forms connected curves crossing the map.
pub const BARRIER_BAND: f32 = 0.055;
/// Movement multiplier inside water (very sluggish, near-impassable).
pub const WATER_MOVE_MULT: f32 = 0.12;
/// Metabolic penalty for being in water (cost of swimming/wading).
pub const WATER_METAB_MULT: f32 = 1.5;
/// Largest `food_mult` across biomes; used to normalize rejection sampling.
pub const BIOME_MAX_FOOD_MULT: f32 = 1.8;
/// Fertility thresholds separating desert | plains | forest | swamp.
pub const BIOME_THRESHOLDS: [f32; 3] = [0.30, 0.60, 0.85];
/// Energy gained by eating one pellet (at full digestion efficiency; the diet
/// niche scales this down, so the base is raised to keep carrying capacity).
pub const FOOD_ENERGY: f32 = 34.0;
pub const FOOD_RADIUS: f32 = 2.5;

// ---- Creatures ----
pub const START_ENERGY: f32 = 60.0;
/// Energy needed before a creature splits in two.
pub const REPRO_ENERGY: f32 = 120.0;
/// Baseline upkeep cost per step (before metabolism gene scaling).
pub const BASE_METABOLISM: f32 = 0.06;
/// Movement cost coefficient: cost = MOVE_COST * speed.
pub const MOVE_COST: f32 = 0.012;
/// Hard cap on population to protect the frame rate (raised for the giant map).
pub const POP_CAP: usize = 12000;

// ---- Diet (continuous carnivory gene) ----
// A creature's carnivory c in 0..1 (decoded from DNA) sets where it sits on the
// food chain. Herbivory efficiency is (1-c); hunting efficiency is c. The
// predator multipliers below are the c=1 endpoints, lerped from 1.0 at c=0.
/// Minimum carnivory before a creature bothers hunting at all.
pub const HUNT_MIN_CARNIVORY: f32 = 0.30;
/// A hunter can catch prey whose carnivory is at least this much lower than its
/// own — i.e. you eat things meaningfully lower on the food chain.
pub const PREY_MARGIN: f32 = 0.20;
/// Carnivory bucket edges for stats/coloring: herbivore | omnivore | carnivore.
pub const DIET_HERBIVORE_MAX: f32 = 0.34;
pub const DIET_CARNIVORE_MIN: f32 = 0.66;

/// Energy a full carnivore gains from catching one prey (scaled by carnivory).
pub const PREDATOR_GAIN: f32 = 30.0;
/// Extra catch radius added to a predator's body radius when hunting.
pub const PREDATOR_CATCH_PAD: f32 = 2.0;
/// Predator upkeep multiplier: they burn energy faster, so they starve when
/// prey get scarce — this is what lets prey rebound instead of going extinct.
pub const PREDATOR_METAB_MULT: f32 = 3.0;
/// Predator speed relative to their gene speed. At 1.0 a fleeing alert prey is
/// effectively uncatchable, so predators rely on ambushing prey that haven't
/// sensed them — which is what keeps the neural arms race in balance. Raising
/// it tips the system toward predators (and tends to wipe out prey).
pub const PREDATOR_SPEED_MULT: f32 = 1.0;

// Phenotype gene ranges (decoded from DNA, value 0..=255 mapped into these).
pub const RADIUS_RANGE: (f32, f32) = (2.5, 7.0);
pub const SPEED_RANGE: (f32, f32) = (0.4, 3.4);
pub const SENSE_RANGE: (f32, f32) = (30.0, 190.0);
/// Metabolism gene multiplies BASE_METABOLISM and scales with body cost.
pub const METAB_RANGE: (f32, f32) = (0.6, 1.8);
/// Longevity gene -> "prime" age (steps) before senescence begins.
pub const LONGEVITY_RANGE: (f32, f32) = (200.0, 1200.0);
/// Memory-leak gene γ for the leaky-integrator hidden state: γ=1 is a plain
/// Elman net (state fully overwritten each step), low γ is a slow integrator
/// that carries state for ~1/γ steps (long-term memory). Floor >0 keeps it live.
pub const LEAK_RANGE: (f32, f32) = (0.15, 1.0);

// ---- Diet niche (food types & specialization) ----
// Each pellet has a "flavor" on a 0..1 niche axis; each biome grows its own
// flavor. A creature's `diet_niche` gene is what it digests best; efficiency
// falls off as a Gaussian with FIXED width, so it physically cannot be good at
// everything — specializing on the local flavor is forced. This makes a
// forest-eater genuinely unfit in the desert, so populations diverge across
// barriers by ecological adaptation (not by any "preference" gene).
/// Tolerance width (sigma) of the diet-efficiency Gaussian. Smaller -> narrower
/// specialists, sharper trade-off. Tuned so same-biome food is eaten near-fully
/// and adjacent-biome food (Δflavor ~0.25) is eaten poorly.
pub const DIET_WIDTH: f32 = 0.18;
/// A creature won't bother eating (or sensing) food it digests below this
/// efficiency, so specialists leave others' food alone -> niches coexist.
/// At this width: home biome ~0.96, adjacent biome (Δ0.26) ~0.35, two biomes
/// away ~0.02 — so neighbours are edible (survivable) but the home flavor is
/// strongly favoured, preserving the specialization gradient.
pub const MIN_EAT_EFF: f32 = 0.15;
/// Random spread of pellet flavor around its biome's flavor.
pub const FOOD_FLAVOR_NOISE: f32 = 0.05;

// ---- Sexual selection ----
/// A showy ornament is a handicap: upkeep is multiplied by (1 + cost·ornament²).
/// The squared term makes the marginal cost rise with ornament size, so a full
/// ornament costs this much extra metabolism while modest ones stay cheap — a
/// nonlinear penalty that bites hardest at the runaway extreme.
pub const ORNAMENT_COST: f32 = 1.0;

// ---- Host–parasite co-evolution (Red Queen) ----
/// Energy drained per step by an infection the host can't resist.
pub const INFECTION_DAMAGE: f32 = 0.28;
/// Matching-allele protection width: a host with `resistance` r is protected
/// against strains near r; protection = exp(-(r-strain)^2 / WIDTH).
pub const PROTECT_WIDTH: f32 = 0.04;
/// Per-contact transmission probability to a healthy neighbour in range.
/// The world is sparse, so contacts are rare — this is kept high so an
/// outbreak can sustain itself at food/biome clusters instead of fizzling.
pub const INFECT_CHANCE: f64 = 0.12;
/// Transmission range (must be < GRID_CELL so the 3x3 query covers it).
pub const INFECT_RADIUS: f32 = 52.0;
/// Strain drift on transmission (the pathogen mutating to escape resistance).
pub const STRAIN_MUT: f32 = 0.04;
/// Per-step chance an infected host clears the infection.
pub const RECOVER_CHANCE: f64 = 0.008;
/// Fraction of founders seeded as infected.
pub const START_INFECTED_FRAC: f32 = 0.1;
/// Environmental reservoir: the dominant circulating strain drifts (random walk
/// per step) so resistance never permanently "wins" — the pathogen escapes.
pub const STRAIN_DRIFT: f32 = 0.004;
/// Per-step chance a healthy host picks up the circulating strain from the
/// environment (keeps the disease from fizzling out in the sparse early world).
pub const BACKGROUND_INFECT: f64 = 0.0004;

// ---- Aging ----
/// Reproduction maturity = prime * this fraction. Long-lived creatures mature
/// later, the cost that keeps the longevity gene from being free.
pub const MATURITY_FRAC: f32 = 0.1;
/// Steps from prime to full decline (senescence factor reaches 1.0).
pub const SENESCENCE_SCALE: f32 = 400.0;
/// Speed lost at full senescence (0.6 -> down to 40% of gene speed).
pub const SENESCENCE_SPEED_DROP: f32 = 0.6;
/// Sense range lost at full senescence.
pub const SENESCENCE_SENSE_DROP: f32 = 0.5;
/// Per-step death probability at full senescence (scaled by senescence²).
pub const AGE_MORTALITY: f64 = 0.006;
/// Predator senescence rate relative to herbivores (1.0 = same). Raising it
/// lets predators live longer, but longer-lived predators over-hunt the
/// age-weakened prey base into a mutual collapse — so it's kept neutral. This is
/// an inherent tension: once aging weakens prey, boosting predators destabilizes.
pub const PREDATOR_LONGEVITY_MULT: f32 = 1.0;
/// Maturity fraction for predators (kept equal to herbivores). Lowering it to
/// help predators breed faster did not save them and risked overshoot collapse.
pub const PREDATOR_MATURITY_FRAC: f32 = 0.1;

/// Max turn (radians per step) applied from the brain's turn output, at full
/// forward drive. Scaled down by throttle, so idle creatures barely turn.
pub const MAX_TURN: f32 = 0.3;

/// Max distance to a fertile same-species partner for sexual reproduction.
/// If none is in range, the creature clones itself (asexual fallback).
pub const MATE_RANGE: f32 = 32.0;

// ---- Genome ----
/// Nucleotides consumed per decoded value (4 -> base-4 number 0..=255).
pub const NT_PER_GENE: usize = 4;
/// Per-nucleotide substitution probability on reproduction.
pub const MUTATION_RATE: f64 = 0.012;
/// Per-nucleotide probability of an insertion, and (separately) a deletion.
/// Indels shift every downstream gene (frameshift), randomizing all downstream
/// weights — so they must be a rare macro-mutation, not a constant load. At this
/// rate the genome (~670 nt) sees ~0.2 indels/child (~18% of offspring get one).
pub const INDEL_RATE: f64 = 0.00015;
/// Genome length is clamped to this band as indels grow/shrink it.
pub const GENOME_MIN_LEN: usize = GENOME_LEN / 2;
pub const GENOME_MAX_LEN: usize = GENOME_LEN * 2;

// ---- Neural network topology ----
// Inputs: food prox/sin/cos, threat prox/sin/cos, neighbor prox/sin/cos,
// heard-signal, own energy, bias.
pub const NN_INPUTS: usize = 12;
pub const NN_HIDDEN: usize = 7;
// Outputs: throttle, turn, signal (emitted call loudness).
pub const NN_OUTPUTS: usize = 3;
/// Decoded weights are mapped into [-WEIGHT_SCALE, WEIGHT_SCALE].
pub const WEIGHT_SCALE: f32 = 4.0;

/// Body-trait genes: radius, speed, sense, metabolism, R, G, B, longevity,
/// carnivory, ornament, preference, resistance, diet_niche, memory-leak γ.
pub const BODY_GENES: usize = 14;

// ---- Marker/tag genome records (brain wiring + body morphology) ----
// After the body genes, the genome holds a single stream of *records*, each
// delimited by a start codon and tagged with a type. One nt-granular scan (any
// reading frame) decodes them, advancing past each record's interior so a record
// can never spawn a spurious nested record. This is indel-robust (an insert/
// delete shifts or drops whole records instead of frameshifting everything) and
// extensible: today's types are synapses (brain wiring) and segments (body plan);
// neuron records (evolvable hidden count) and body-grown ports come later.
//
/// Start codon (nt triplet) marking the head of any record.
pub const RECORD_START: [u8; 3] = [3, 3, 2]; // T,T,G
/// The gene right after the start codon selects the record type. A *segment*
/// record needs that gene at or above this high threshold; everything else is a
/// *synapse*. Segments are therefore a rare slice of type-gene space, so a body
/// plan only changes by a rare macro-mutation (and never by a synapse record's
/// type gene drifting one step) — selection then has to amplify it.
pub const SEGMENT_TYPE_MIN: u8 = 240; // ~6% of type-gene values
/// nt consumed by a synapse record: start(3) + type + src + dst + weight.
pub const SYNAPSE_RECORD_NT: usize = 3 + NT_PER_GENE + 3 * NT_PER_GENE; // 19
/// nt consumed by a segment record: start(3) + type + length + width + appendage
/// + flexibility.
pub const SEGMENT_RECORD_NT: usize = 3 + NT_PER_GENE + 4 * NT_PER_GENE; // 23

// Brain port tags.
/// Source ports a synapse may read from: the inputs, then the hidden units.
pub const SRC_PORTS: usize = NN_INPUTS + NN_HIDDEN; // 19
/// Destination ports a synapse may drive: the hidden units, then the outputs.
pub const DST_PORTS: usize = NN_HIDDEN + NN_OUTPUTS; // 10
/// Founder brain = a dense connection set (every input->hidden, hidden->hidden,
/// hidden->output) emitted as that many synapse records.
pub const FOUNDER_SYNAPSES: usize =
    NN_INPUTS * NN_HIDDEN + NN_HIDDEN * NN_HIDDEN + NN_HIDDEN * NN_OUTPUTS; // 154

// ---- Vertical layers ----
// The world has a small stack of layers. A creature's layer is its morphological
// stratum: wings put it in the air, a burrow appendage underground, otherwise the
// surface. Surface dwellers forage the positioned food pellets exactly as before
// (baseline carrying capacity unchanged). Each non-surface layer instead offers a
// fixed foraging *capacity* split among its current occupants — density-dependent,
// so the niche self-limits (rich when sparse, poor when crowded) and can't run
// away. Sensing/eating/hunting are gated to a creature's layer, so a non-surface
// layer is also a refuge from surface predators. This is the selection pressure
// that finally makes the burrow/wing appendages pay.
pub const N_LAYERS: usize = 3;
pub const LAYER_UNDERGROUND: u8 = 0;
pub const LAYER_SURFACE: u8 = 1;
pub const LAYER_AIR: u8 = 2;
/// Total benthic (underground) foraging energy available per step, split among
/// the creatures currently underground (reachable only with a burrow appendage).
pub const BENTHIC_CAPACITY: f32 = 900.0;
/// Total aerial foraging energy per step, split among winged occupants.
pub const AERIAL_CAPACITY: f32 = 450.0;

// ---- Body morphology (segment chain) ----
// A body is a chain of segments decoded from segment records. Founders emit none
// (a single implicit segment sized by the radius gene == the old circle); chains
// then grow by mutation. Appendages (fin/wing/leg/burrow) are decoded now and
// drive medium locomotion + layer access in later Phase-2 sub-steps.
/// Decoded segment-chain length is capped here (perf + sanity vs runaway indels).
pub const MAX_SEGMENTS: usize = 8;
/// Per-segment length and width gene ranges (px).
pub const SEG_LEN_RANGE: (f32, f32) = (2.0, 7.0);
pub const SEG_WIDTH_RANGE: (f32, f32) = (1.5, 5.0);
/// Number of appendage kinds (None, Fin, Wing, Leg, Burrow).
pub const APPENDAGE_KINDS: usize = 5;
/// Metabolic upkeep added per body segment, as a fraction of the body cost
/// multiplier. Counterweight to the locomotor benefit of extra segments, so the
/// evolved chain length settles at an interior optimum instead of the cap.
pub const SEGMENT_UPKEEP: f32 = 0.10;
/// Extra upkeep per appendage (fins/wings/legs cost to grow and carry).
pub const APPENDAGE_UPKEEP: f32 = 0.08;

/// Canonical genome length (nt): fixed body-gene block + the founder's synapse
/// records. Indels then push length around within the clamp band below.
pub const GENOME_LEN: usize =
    BODY_GENES * NT_PER_GENE + FOUNDER_SYNAPSES * SYNAPSE_RECORD_NT;
