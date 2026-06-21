//! Stream-power landscape evolution (LEM) — the geomorphic realism pass.
//!
//! Tectonics + noise give a macro surface, but mountains shaped by noise alone lack the
//! signature of real ranges: **dendritic valley networks** and **concave river profiles**.
//! This pass couples the plate UPLIFT rate `U` (from [`crate::tectonics`], the distance-
//! weighted convergence, NOT the static elevation) to FLUVIAL INCISION over geologic time:
//!
//! ```text
//!   ∂h/∂t = U − K·A^m·S^n + D·∇²h
//!           └up┘  └ fluvial ┘  └ hillslope diffusion ┘
//! ```
//!
//! `A` = upstream drainage area (flow accumulation), `S` = slope to the receiver. Where flow
//! concentrates (large `A`) the river incises a concave valley; uplift keeps the belts standing
//! so the system tends toward the erosional steady state that spaces ranges and carves trunk-
//! and-tributary networks. Implicit **Braun-Willett (2013)** stepping (`n = 1` closed-form):
//! one O(n) downstream→upstream sweep per step, unconditionally stable at large `dt`.
//!
//! Run as a REFINEMENT on the existing elevation (not a replacement), so the continents /
//! shelf / land-water balance the rest of worldgen depends on are preserved while the belts
//! gain real drainage. The droplet pass that follows then adds fine channel detail.
//!
//! Cost: the per-step flow routing (priority-flood) dominates, so it is recomputed only every
//! `ROUTE_EVERY` steps (drainage changes slowly); the cheap uplift/incision/diffusion run
//! every step. Deterministic — no RNG, fixed iteration count, fixed tie-breaking in the flood.

use crate::config::*;
use rayon::prelude::*;

/// LEM iterations. Enough for incision to propagate headward through the belts.
const STEPS: u32 = 24;
/// Recompute flow routing (priority-flood + drainage area) every N steps — drainage geometry
/// changes slowly, and the flood (the per-step cost) is serial, so amortise it.
const ROUTE_EVERY: u32 = 6;
const DT: f32 = 1.0;
/// Uplift applied per unit time, scaling the tectonic `U(x)` field (`[0,1]`). Keeps the belts
/// standing against incision so relief tends to a steady state instead of eroding flat.
const UPLIFT_RATE: f32 = 0.0020;
/// Erodibility. `K·dt·A^m` is the implicit incision weight `f`: on hillslopes (A≈1) `f≈K·dt`
/// (negligible — slopes don't channelise), on trunks (A^m in the hundreds–thousands) `f≫1`
/// (the channel cuts toward its receiver). Tuned so the gradient reads as a dendritic network.
const K: f32 = 0.0030;
const M: f32 = 0.5;
// n = 1 ⇒ the implicit incision is the closed form below (no Newton iteration needed).
/// Hillslope diffusion (explicit Laplacian). Smooths ridge-to-channel transitions and keeps
/// incision from leaving knife walls. Kept well under the 0.25 explicit-stability limit.
const DIFFUSE: f32 = 0.12;

/// Refine `elev` in place: couple tectonic uplift to stream-power fluvial incision so the
/// belts develop dendritic valleys and concave profiles. `uplift` is the `[0,1]` rate field
/// from [`crate::tectonics::TectonicField::uplift_field`].
pub fn refine(elev: &mut [f32], uplift: &[f32]) {
    let n = COLS * ROWS;
    let mut area = vec![1.0f32; n];
    let mut receiver = vec![0u32; n];
    let mut order: Vec<u32> = Vec::new();
    let mut delta = vec![0f32; n]; // diffusion scratch, reused every step
    for step in 0..STEPS {
        // 1. Flow routing (amortised): priority-flood gives depression-free receivers + the
        //    downstream→upstream order; accumulate drainage area in reverse.
        if step % ROUTE_EVERY == 0 {
            let (_filled, recv, ord) = crate::hydrology::priority_flood(elev);
            receiver = recv;
            order = ord;
            for a in area.iter_mut() {
                *a = 1.0;
            }
            for &iu in order.iter().rev() {
                let i = iu as usize;
                let r = receiver[i] as usize;
                if r != i {
                    area[r] += area[i];
                }
            }
        }
        // 2. Uplift: raise the land by the tectonic rate (belts most, plains least). Per-cell
        //    independent ⇒ parallel, bit-identical (no reduction).
        elev.par_iter_mut().zip(uplift.par_iter()).for_each(|(e, u)| {
            *e += DT * UPLIFT_RATE * *u;
        });
        // 3. Implicit stream-power incision, downstream→upstream so each receiver is already
        //    updated. With n=1 and unit receiver distance (4-connected):
        //        h_i ← (h_i + f·h_recv) / (1 + f),   f = K·dt·A_i^m
        //    Skip outlets (receiver = self) and cells sitting in a filled flat / lake (no
        //    downhill on the real surface ⇒ nothing to incise).
        for &iu in order.iter() {
            let i = iu as usize;
            let r = receiver[i] as usize;
            if r == i {
                continue;
            }
            let hr = elev[r];
            if elev[i] <= hr {
                continue;
            }
            let f = K * DT * area[i].powf(M);
            elev[i] = (elev[i] + f * hr) / (1.0 + f);
        }
        // 4. Hillslope diffusion: explicit 4-neighbour Laplacian into a delta buffer, applied
        //    after so it reads a consistent surface. Smooths the ridge-to-channel transition.
        diffuse(elev, &mut delta);
    }
    for h in elev.iter_mut() {
        *h = h.clamp(0.0, 1.0);
    }
}

/// One explicit hillslope-diffusion step: `h += DIFFUSE·∇²h` (4-neighbour Laplacian, clamped
/// edges). Writes into the reused `delta` scratch (every cell independent ⇒ parallel,
/// bit-identical), then applies it — so every cell reads the same pre-step surface.
fn diffuse(elev: &mut [f32], delta: &mut [f32]) {
    let w = COLS;
    delta.par_chunks_mut(COLS).enumerate().for_each(|(y, drow)| {
        let (ym, yp) = (y.saturating_sub(1), (y + 1).min(ROWS - 1));
        for x in 0..w {
            let c = elev[y * w + x];
            let (xm, xp) = (x.saturating_sub(1), (x + 1).min(w - 1));
            let lap = (elev[y * w + xm] - c)
                + (elev[y * w + xp] - c)
                + (elev[ym * w + x] - c)
                + (elev[yp * w + x] - c);
            drow[x] = DIFFUSE * lap;
        }
    });
    elev.par_iter_mut().zip(delta.par_iter()).for_each(|(e, d)| *e += *d);
}
