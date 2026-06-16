//! Rolling history of population/trait averages for the live trend graph.

const HISTORY: usize = 600;

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
