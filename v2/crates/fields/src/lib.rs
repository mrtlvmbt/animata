//! `fields` — CPU `FieldStore` backend with TWO field classes (M2, doc 14 §1):
//!
//! * **conserved** resource field — fixed-point INTEGER, in the energy balance, exact conservation,
//!   thread-count-independent (integer-associative merge, R14). Flux diffusion (§5.1) with an explicit
//!   fixed-point CFL coefficient `k = round(α·2^F)`.
//! * **signal** field — `f32` pheromone, NOT in the balance, separable-blur + multiplicative decay,
//!   deterministic only under a fixed serial reduction order (F2 — no parallel float reduction).
//!
//! `commit_merge` folds per-thread deposit batches: the `Canonical` strategy (sort by Morton→Entity,
//! integer sum) is thread-count-independent for the conserved layer; the `NonAssociative` strategy is
//! the deliberately-broken negative path for the R14 teeth test.

use sim_core::{fnv_mix, morton2, Deposit, FieldStore, MergeStrategy, Vec2Fixed, FNV_OFFSET};

/// Compute the fixed-point flux coefficient `k = round(α·2^F)` for `α = α_num/α_den ∈ (0,¼]`.
/// `round` (NOT `floor`): `floor(α·2^F)` is 0 for small α → a DEAD solver (doc 14 §5.1 / F4).
pub fn flux_k_from_alpha(alpha_num: i64, alpha_den: i64, f: u32) -> i64 {
    let scaled = alpha_num << f; // α_num · 2^F
    (scaled + alpha_den / 2) / alpha_den // round-to-nearest
}

/// CPU field backend.
pub struct CpuFieldStore {
    m_field: i64,
    dim: i64,
    grid_w: i64,
    grid_h: i64,
    /// Cell indices in Morton order — the canonical diffusion pair-traversal order (doc 14 §5.1).
    morton_order: Vec<usize>,
    n_layers: usize,

    // conserved (integer); outer index = layer, inner index = cell
    conserved: Vec<Vec<i64>>,
    conserved_staging: Vec<Vec<i64>>,
    caps: Vec<Vec<i64>>,
    regen_rates: Vec<i64>,
    flux_ks: Vec<i64>,
    flux_f: u32,

    // signal (f32) — unchanged; single layer
    signal: Vec<f32>,
    signal_staging: Vec<f32>,
    signal_tmp: Vec<f32>,
    decay: f32,
}

impl CpuFieldStore {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        dim: i64,
        m_field: i64,
        caps: Vec<i64>,
        regen_rate: i64,
        flux_k: i64,
        flux_f: u32,
        decay: f32,
    ) -> Self {
        let grid_w = dim / m_field;
        let grid_h = dim / m_field;
        let n = (grid_w * grid_h) as usize;
        assert_eq!(caps.len(), n, "caps must cover the grid");
        let conserved_l0: Vec<i64> = caps.iter().map(|c| c / 2).collect();
        let mut morton_order: Vec<usize> = (0..n).collect();
        morton_order.sort_by_key(|&i| {
            let cx = (i as i64 % grid_w) as u32;
            let cz = (i as i64 / grid_w) as u32;
            morton2(cx, cz)
        });
        CpuFieldStore {
            m_field,
            dim,
            grid_w,
            grid_h,
            morton_order,
            n_layers: 1,
            conserved: vec![conserved_l0],
            conserved_staging: vec![vec![0; n]],
            caps: vec![caps],
            regen_rates: vec![regen_rate],
            flux_ks: vec![flux_k],
            flux_f,
            signal: vec![0.0; n],
            signal_staging: vec![0.0; n],
            signal_tmp: vec![0.0; n],
            decay,
        }
    }

    /// Multi-layer constructor for `L > 1`. `caps_per_layer[i]` is the per-cell cap for layer `i`;
    /// all layers share the same grid. Each layer has its own `regen_rates[i]` and `flux_ks[i]`;
    /// `flux_f` is shared. A-4 uses this to build the L=3 scenario.
    #[allow(clippy::too_many_arguments)]
    pub fn new_layered(
        dim: i64,
        m_field: i64,
        caps_per_layer: Vec<Vec<i64>>,
        regen_rates: Vec<i64>,
        flux_ks: Vec<i64>,
        flux_f: u32,
        decay: f32,
    ) -> Self {
        let n_layers = caps_per_layer.len();
        assert!(n_layers >= 1, "must have at least one layer");
        let grid_w = dim / m_field;
        let grid_h = dim / m_field;
        let n = (grid_w * grid_h) as usize;
        for (i, caps) in caps_per_layer.iter().enumerate() {
            assert_eq!(caps.len(), n, "layer {i} caps must cover the grid");
        }
        assert_eq!(regen_rates.len(), n_layers, "one regen_rate per layer");
        assert_eq!(flux_ks.len(), n_layers, "one flux_k per layer");
        let conserved: Vec<Vec<i64>> = caps_per_layer.iter()
            .map(|caps| caps.iter().map(|c| c / 2).collect())
            .collect();
        let mut morton_order: Vec<usize> = (0..n).collect();
        morton_order.sort_by_key(|&i| {
            let cx = (i as i64 % grid_w) as u32;
            let cz = (i as i64 / grid_w) as u32;
            morton2(cx, cz)
        });
        CpuFieldStore {
            m_field,
            dim,
            grid_w,
            grid_h,
            morton_order,
            n_layers,
            conserved,
            conserved_staging: (0..n_layers).map(|_| vec![0; n]).collect(),
            caps: caps_per_layer,
            regen_rates,
            flux_ks,
            flux_f,
            signal: vec![0.0; n],
            signal_staging: vec![0.0; n],
            signal_tmp: vec![0.0; n],
            decay,
        }
    }

    #[inline]
    fn cell_coords(&self, pos: Vec2Fixed) -> (i64, i64) {
        (pos.0.rem_euclid(self.dim) / self.m_field, pos.1.rem_euclid(self.dim) / self.m_field)
    }

    #[inline]
    fn idx(&self, cx: i64, cz: i64) -> usize {
        (cz.rem_euclid(self.grid_h) * self.grid_w + cx.rem_euclid(self.grid_w)) as usize
    }

    /// No-flux (reflective) index: clamps out-of-range coordinates to the boundary cell instead of
    /// wrapping. Used by gradient sampling to match the diffusion boundary (M1/F5): both use the
    /// same no-flux convention so chemotaxis and transport agree at the domain edge.
    #[inline]
    fn idx_nf(&self, cx: i64, cz: i64) -> usize {
        let cx = cx.clamp(0, self.grid_w - 1);
        let cz = cz.clamp(0, self.grid_h - 1);
        (cz * self.grid_w + cx) as usize
    }

    fn regenerate(&mut self) -> i64 {
        let mut injected = 0;
        for layer in 0..self.n_layers {
            for (g, cap) in self.conserved[layer].iter_mut().zip(self.caps[layer].iter()) {
                let inj = self.regen_rates[layer].min((*cap - *g).max(0));
                *g += inj;
                injected += inj;
            }
        }
        injected
    }

    /// Integer flux diffusion (doc 14 §5.1). Pairs traversed in Morton order; per pair
    /// `num=(u_a−u_b)·k`, `flux=num>>F` (arithmetic shift → the un-shifted remainder stays in `a`),
    /// clamped so neither cell goes negative (`Σ|flux|≤u_a`). Σ conserved EXACTLY.
    fn diffuse_conserved(&mut self) {
        for layer in 0..self.n_layers {
            for k in 0..self.morton_order.len() {
                let a = self.morton_order[k];
                let cx = a as i64 % self.grid_w;
                let cz = a as i64 / self.grid_w;
                // right and down neighbours (reflective/no-flux boundary — no wrap).
                if cx + 1 < self.grid_w {
                    let b = self.idx(cx + 1, cz);
                    self.exchange_layer(layer, a, b);
                }
                if cz + 1 < self.grid_h {
                    let b = self.idx(cx, cz + 1);
                    self.exchange_layer(layer, a, b);
                }
            }
        }
    }

    #[inline]
    fn exchange_layer(&mut self, layer: usize, a: usize, b: usize) {
        let num = (self.conserved[layer][a] - self.conserved[layer][b]) * self.flux_ks[layer];
        let mut flux = num >> self.flux_f; // arithmetic shift; remainder stays in `a`
        // Local boundedness: outgoing ≤ source content ⇒ no negative cells.
        if flux > 0 {
            flux = flux.min(self.conserved[layer][a]);
        } else if flux < 0 {
            flux = -((-flux).min(self.conserved[layer][b]));
        }
        self.conserved[layer][a] -= flux;
        self.conserved[layer][b] += flux;
    }

    /// Separable blur ([1,2,1]/4 ×2) + multiplicative decay — per-cell independent gather (read the
    /// current version, write the other), so it is deterministic and reduction-free (F2).
    fn blur_decay_signal(&mut self) {
        // horizontal pass: signal → tmp
        for cz in 0..self.grid_h {
            for cx in 0..self.grid_w {
                let l = self.signal[self.idx(cx - 1, cz)];
                let c = self.signal[self.idx(cx, cz)];
                let r = self.signal[self.idx(cx + 1, cz)];
                let i = self.idx(cx, cz);
                self.signal_tmp[i] = 0.25 * l + 0.5 * c + 0.25 * r;
            }
        }
        // vertical pass + decay: tmp → signal
        let keep = 1.0 - self.decay;
        for cz in 0..self.grid_h {
            for cx in 0..self.grid_w {
                let u = self.signal_tmp[self.idx(cx, cz - 1)];
                let c = self.signal_tmp[self.idx(cx, cz)];
                let d = self.signal_tmp[self.idx(cx, cz + 1)];
                let i = self.idx(cx, cz);
                self.signal[i] = (0.25 * u + 0.5 * c + 0.25 * d) * keep;
            }
        }
    }
}

impl FieldStore for CpuFieldStore {
    fn m_field(&self) -> i64 {
        self.m_field
    }

    fn cell_index(&self, pos: Vec2Fixed) -> usize {
        let (cx, cz) = self.cell_coords(pos);
        self.idx(cx, cz)
    }

    fn cell_morton(&self, pos: Vec2Fixed) -> u32 {
        let (cx, cz) = self.cell_coords(pos);
        morton2(cx as u32, cz as u32)
    }

    fn check_meta(&self, expected_m_field: i64) -> Result<(), String> {
        if self.m_field == expected_m_field {
            Ok(())
        } else {
            Err(format!("M_field mismatch: field={} expected={}", self.m_field, expected_m_field))
        }
    }

    // ── conserved ───────────────────────────────────────────────────────────────────────────────
    fn conserved_at(&self, pos: Vec2Fixed, layer: usize) -> i64 {
        self.conserved[layer][self.cell_index(pos)]
    }

    fn conserved_gradient(&self, pos: Vec2Fixed, range: i64, layer: usize) -> (i64, i64) {
        let (cx, cz) = self.cell_coords(pos);
        // No-flux boundary (M1/F5): clamp neighbours to edge cell instead of toroidal wrap.
        // Matches diffuse_conserved (which already guards cx+1 < grid_w) so chemotaxis and
        // transport use the same boundary convention.
        let gx = self.conserved[layer][self.idx_nf(cx + range, cz)]
            - self.conserved[layer][self.idx_nf(cx - range, cz)];
        let gz = self.conserved[layer][self.idx_nf(cx, cz + range)]
            - self.conserved[layer][self.idx_nf(cx, cz - range)];
        (gx, gz)
    }

    fn conserved_take(&mut self, pos: Vec2Fixed, amount: i64, layer: usize) -> i64 {
        let i = self.cell_index(pos);
        let got = amount.min(self.conserved[layer][i]).max(0);
        self.conserved[layer][i] -= got;
        got
    }

    fn deposit_conserved(&mut self, cell: usize, amount: i64, layer: usize) {
        debug_assert!(layer < self.n_layers, "deposit layer {} >= n_layers {}", layer, self.n_layers);
        self.conserved_staging[layer][cell] += amount;
    }

    fn conserved_total(&self, layer: usize) -> i64 {
        self.conserved[layer].iter().sum()
    }

    fn conserved_total_all(&self) -> i64 {
        self.conserved.iter().flat_map(|l| l.iter()).sum()
    }

    fn conserved_hash(&self) -> u64 {
        // At L=1: folds self.conserved[0] identically to the pre-A1 flat fold — no layer index
        // or separator mixed in. At L>1: appends each subsequent layer's cells in order.
        let mut h = FNV_OFFSET;
        for layer_data in &self.conserved {
            for &v in layer_data {
                h = fnv_mix(h, v as u64);
            }
        }
        h
    }

    // ── signal ──────────────────────────────────────────────────────────────────────────────────
    // signal_at (bilinear sample) removed — M2/F3: no consumer in the tick loop.
    // signal_gradient removed — M3/F3: dead per-tick f32 compute (integer brain never read it).
    // Both may return when a real consumer lands; the underlying signal grid is still maintained.

    fn signal_total(&self) -> f32 {
        // SERIAL reduction in cell order (no parallel float fold — F2).
        let mut s = 0.0f32;
        for &v in &self.signal {
            s += v;
        }
        s
    }

    fn signal_hash(&self) -> u64 {
        let mut h = FNV_OFFSET;
        for &v in &self.signal {
            h = fnv_mix(h, v.to_bits() as u64);
        }
        h
    }

    fn signal_all_finite(&self) -> bool {
        self.signal.iter().all(|v| v.is_finite())
    }

    // ── scatter + solver ──────────────────────────────────────────────────────────────────────────
    fn commit_merge(&mut self, batches: &[Vec<Deposit>], strategy: MergeStrategy) {
        match strategy {
            MergeStrategy::Canonical => {
                // Flatten → sort by (layer, Morton, Entity) → apply in that single serial order.
                // Integer conserved sum is associative ⇒ identical for any thread/layer count (R14).
                // Signal stays flat (single buffer keyed by cell — no per-layer signal in slice A).
                let mut all: Vec<&Deposit> = batches.iter().flatten().collect();
                all.sort_by_key(|d| (d.layer, d.morton, d.entity_bits));
                for d in all {
                    debug_assert!(d.layer < self.n_layers, "deposit layer {} >= n_layers {}", d.layer, self.n_layers);
                    self.conserved_staging[d.layer][d.cell] += d.conserved;
                    self.signal_staging[d.cell] += d.signal; // signal is NOT layered in slice A
                }
            }
            MergeStrategy::NonAssociative => {
                // NEGATIVE path (R14 teeth): per-batch, per-layer partial conserved sums, then fold the
                // N partials with a non-associative, COUNT-sensitive combine. N=1 → the correct sum;
                // N>1 → a different value ⇒ the 1-vs-N conserved hash differs ⇒ R14 goes RED.
                let n = self.conserved_staging[0].len();
                for layer in 0..self.n_layers {
                    let partials: Vec<Vec<i64>> = batches
                        .iter()
                        .map(|b| {
                            let mut p = vec![0i64; n];
                            for d in b {
                                if d.layer == layer {
                                    p[d.cell] += d.conserved;
                                }
                            }
                            p
                        })
                        .collect();
                    for (cell, slot) in self.conserved_staging[layer].iter_mut().enumerate() {
                        let mut acc = 0i64;
                        for p in &partials {
                            acc = acc.rotate_left(1).wrapping_add(p[cell]); // non-associative
                        }
                        *slot += acc;
                    }
                }
                // signal handled canonically (irrelevant to the conserved-only R14 assert).
                let mut all: Vec<&Deposit> = batches.iter().flatten().collect();
                all.sort_by_key(|d| (d.layer, d.morton, d.entity_bits));
                for d in all {
                    self.signal_staging[d.cell] += d.signal;
                }
            }
        }
    }

    fn solve(&mut self) -> i64 {
        // Apply staged deposits → grid (the t+1 snapshot, R17) for each layer.
        // P2-R-B: clamp conserved ≥ 0 (O₂ consumption can produce negative staging;
        // shortfall → clamp-to-0, no mass creation/destruction, exact conservation).
        for layer in 0..self.n_layers {
            for (g, s) in self.conserved[layer].iter_mut().zip(self.conserved_staging[layer].iter_mut()) {
                *g = (*g + *s).max(0);
                *s = 0;
            }
        }
        for (g, s) in self.signal.iter_mut().zip(self.signal_staging.iter_mut()) {
            *g += *s;
            *s = 0.0;
        }
        let injected = self.regenerate();
        self.diffuse_conserved();
        self.blur_decay_signal();
        injected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(flux_k: i64) -> CpuFieldStore {
        CpuFieldStore::new(8, 1, vec![100; 64], 5, flux_k, 16, 0.05)
    }

    #[test]
    fn flux_k_uses_round_not_floor() {
        // α = 1/100000 with F=16: round → 1 (live solver), floor → 0 (dead). round MUST be used (F4).
        let k_round = flux_k_from_alpha(1, 100_000, 16);
        let k_floor = (1i64 << 16) / 100_000;
        assert_eq!(k_round, 1);
        assert_eq!(k_floor, 0);
        // α = 1/4 → 16384.
        assert_eq!(flux_k_from_alpha(1, 4, 16), 16384);
    }

    #[test]
    fn diffusion_nonzero_and_conserved_at_both_alpha_edges() {
        for k in [1, 16384] {
            let mut f = field(k);
            // A big spike so even the near-0 α (k=1) transfers ≥1 integer unit.
            let spike = f.idx(3, 3);
            f.conserved[0][spike] += 200_000;
            let before = f.conserved_total(0);
            let snapshot = f.conserved[0].clone();
            f.diffuse_conserved();
            assert_eq!(f.conserved_total(0), before, "Σ must be exactly conserved (k={k})");
            assert_ne!(f.conserved[0], snapshot, "diffusion must be non-zero (k={k})");
            assert!(f.conserved[0].iter().all(|&v| v >= 0), "no negative cells (k={k})");
        }
    }

    #[test]
    fn signal_blur_decay_is_finite_and_shrinks_total() {
        let mut f = field(16384);
        let c = f.idx(4, 4);
        f.signal[c] = 100.0;
        let t0 = f.signal_total();
        f.blur_decay_signal();
        assert!(f.signal_all_finite());
        assert!(f.signal_total() < t0, "decay must shrink total concentration");
    }

    // A-3 (R15): ledger conservation across all layers. 3-layer store, caps=60 → start=30
    // per cell, 4×4 grid (16 cells). correct initial = 3×16×30 = 1440; layer-0-only = 480.
    // No agents, no ticks — tests the counting invariant only.
    #[test]
    fn ledger_r15_sums_all_layers() {
        let caps = vec![60i64; 16];
        let f = CpuFieldStore::new_layered(
            4, 1,
            vec![caps.clone(), caps.clone(), caps.clone()],
            vec![1, 1, 1],
            vec![0, 0, 0], // no diffusion needed for this counting test
            16,
            0.0,
        );
        let initial_all = f.conserved_total_all();
        let initial_l0 = f.conserved_total(0);
        // R15: initial_all − current_all == 0 (no ticks, state unchanged)
        assert_eq!(initial_all - f.conserved_total_all(), 0, "conserved_total_all gives R15 == 0");
        // Teeth: layer-0-only under-counts → residual ≠ 0
        assert_ne!(initial_l0 - f.conserved_total_all(), 0, "layer-0-only under-counts R15 (teeth)");
        assert!(initial_l0 < initial_all, "layer-0 total < all-layers total at L=3");
    }

    // A-5: closed-form per-layer regen growth. With uniform start (= caps/2) and per-cell headroom
    // cap/2 > K·regen[l] (no saturation), diffusion of a uniform field moves zero net mass, so
    // conserved_total(l) grows by EXACTLY K · n · regen[l] over K solve ticks.
    // Reds on: inert layer (regen=0) → Δ=0; shared global regen → per-layer Δ doesn't match each
    // regen[l]; saturation → equality breaks. No agents, no noise → integer-only, x86.
    #[test]
    fn per_layer_regen_growth_exact() {
        const K: i64 = 10;
        const N: usize = 16; // 4×4 grid
        let regen = [5i64, 3, 1]; // distinct, all > 0
        // caps = 200 → start = 100; after K ticks max cell = 100 + K*regen[l] ≤ 150 < 200 (no sat.)
        let caps = vec![200i64; N];
        let flux_k = flux_k_from_alpha(1, 8, 16); // live solver; uniform field → zero net diffusion
        let mut f = CpuFieldStore::new_layered(
            4, 1,
            vec![caps.clone(), caps.clone(), caps.clone()],
            regen.to_vec(),
            vec![flux_k, flux_k, flux_k],
            16,
            0.0,
        );
        let before = [f.conserved_total(0), f.conserved_total(1), f.conserved_total(2)];
        for _ in 0..K {
            f.solve();
        }
        for l in 0..3 {
            let expected_delta = K * N as i64 * regen[l];
            assert_eq!(
                f.conserved_total(l) - before[l],
                expected_delta,
                "layer {l}: expected delta K*n*regen={expected_delta}",
            );
        }
    }
}
