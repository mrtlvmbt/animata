//! Rolling history of population/trait averages for the live trend graph.

const HISTORY: usize = 600;

// Observability DTO: every field is populated each tick, but some are consumed
// only by the `dev` JSON bridge (`dev_bridge`) and the headless tuning example,
// not by the default windowed build — so the default bin sees them as write-only.
#[allow(dead_code)]
#[derive(Clone, Copy, Default)]
pub struct Snapshot {
    pub population: usize,
    pub herbivores: usize,
    pub predators: usize,
    pub avg_speed: f32,
    pub avg_sense: f32,
    pub avg_radius: f32,
    pub avg_metabolism: f32,
    /// Mean carnivory (0 = all herbivores, 1 = all carnivores).
    pub avg_carnivory: f32,
    /// Mean sexual-display ornament (rises under Fisherian runaway).
    pub avg_ornament: f32,
    /// Mean emitted signal loudness (communication / alarm calls).
    pub avg_signal: f32,
    /// Mean disease-resistance allele (tracks the Red Queen chase).
    pub avg_resistance: f32,
    /// Fraction of the population currently infected (0..1).
    pub infected_frac: f32,
    /// Mean realized recurrent-memory reliance (0..1): the recurrent term's
    /// share of hidden activation while behaving, averaged over the population.
    pub avg_memory: f32,
    /// Mean diet niche (0..1): the food flavor the population digests best.
    pub avg_niche: f32,
    /// Mean body-segment count (0 == single implicit segment); rises as evolvable
    /// morphologies grow chains.
    pub avg_segments: f32,
    /// Fraction of the population carrying at least one appendage (fin/wing/leg/
    /// burrow) — the share that has moved off the plain circular body plan.
    pub appendaged_frac: f32,
    /// Fraction of the population living underground (burrowers) and in the air
    /// (fliers) — the vertical niches, populated as those body plans evolve.
    pub frac_underground: f32,
    pub frac_air: f32,
    /// Mean hidden-neuron count (evolvable brain width).
    pub avg_hidden: f32,
    /// Fraction of the population bearing a fin (the aquatic-forager niche).
    pub frac_finned: f32,
    /// Std-dev of diet niche: rises and goes bimodal as the population splits
    /// into food specialists — the live signal of ecological speciation.
    pub niche_spread: f32,
    /// Mean per-trait std-dev across the population (0 = monoculture).
    pub diversity: f32,
    /// Count of distinct surviving founder lineages (drops as clades die out).
    pub lineages: usize,
    /// Count of detected phenotype species clusters.
    pub species: usize,
    pub max_generation: u32,
    // ---- Marker substrate (emergent signalling) instrumentation ----
    /// Mean total scent emission per creature (is anyone signalling?).
    pub marker_emit: f32,
    /// Fraction of the population with at least one marker-tuned receptor organ
    /// (is anyone *listening*? — the co-evolution indicator).
    pub marker_listener_frac: f32,
    /// Per-channel emergent meaning: correlation between a channel's local
    /// intensity and the creature's food proximity. |r| above ~0 means the channel
    /// carries information about food — a sign of self-organised semantics.
    pub channel_meaning: [f32; crate::config::N_MARKER_CHANNELS],
    /// Mean colour↔biome-tint contrast (RGB distance). Falls as camouflage
    /// (crypsis) evolves — bodies coming to match their biome.
    pub avg_color_contrast: f32,
    /// Same, but for carnivores only — to see if predators evolve *ambush*
    /// camouflage (so prey don't spot them) even when prey don't.
    pub avg_color_contrast_pred: f32,
}

/// Top lineages (id, count) at a snapshot, for the Muller stacked-area plot.
pub type LineageRow = Vec<(u32, u32)>;

pub struct Stats {
    /// Most recent snapshot last; capped at HISTORY entries.
    pub history: Vec<Snapshot>,
    /// Per-snapshot top-lineage counts, in lockstep with `history`.
    pub lineage_history: Vec<LineageRow>,
}

impl Stats {
    pub fn new() -> Self {
        Stats {
            history: Vec::with_capacity(HISTORY),
            lineage_history: Vec::with_capacity(HISTORY),
        }
    }

    /// Push a snapshot plus its top-lineage row, keeping both series aligned.
    pub fn push(&mut self, s: Snapshot, lineages: LineageRow) {
        if self.history.len() == HISTORY {
            self.history.remove(0);
            self.lineage_history.remove(0);
        }
        self.history.push(s);
        self.lineage_history.push(lineages);
    }

    pub fn latest(&self) -> Snapshot {
        self.history.last().copied().unwrap_or_default()
    }
}
