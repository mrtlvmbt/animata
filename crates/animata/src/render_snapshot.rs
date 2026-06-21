//! The render-side snapshot of the sim — the read seam between the simulation and the renderer.
//!
//! The world render pass (and interactive picking) read ONLY this, never `Sim` directly. That makes
//! the data surface the renderer needs explicit and cheap, so the sim can later move to its own thread
//! (Phase B) and publish snapshots without the renderer reaching into live state.
//!
//! DISCIPLINE: cheap per-creature data only — **no genome clones**. The developed `body` (morphology)
//! is the one heavy field, included solely for creatures near the camera at high zoom, where it's
//! actually drawn; everything else is the dot/mat LOD which needs only pos/coloration/biomass.

use animata_sim::sim::Sim;
use macroquad::math::Vec2;

/// Per-creature data the world render pass needs (~24 bytes + the optional body).
pub struct CreatureDot {
    pub id: u64,
    pub pos: Vec2,
    pub coloration: f32,
    pub biomass: u32,
    /// Developed body as lattice cells `(dx, dy, cell_type)` — `Some` only for near-camera creatures
    /// at high zoom (the morphology LOD); `None` otherwise (the dot/mat LODs don't need it).
    pub body: Option<Vec<(i16, i16, u8)>>,
}

/// What the renderer reads each frame instead of touching `Sim`.
pub struct RenderSnapshot {
    /// Sim tick the snapshot was taken at. Read once the sim runs on its own thread (Phase B) to label
    /// the snapshot independently of the main-thread clock; unused in the single-threaded Phase A seam.
    #[allow(dead_code)]
    pub tick: u64,
    pub creatures: Vec<CreatureDot>,
}

impl RenderSnapshot {
    /// Build from the live sim. `body_near = Some((center_xz, half_extent))` requests body layouts for
    /// creatures within that world-space AABB (the high-zoom morphology LOD); `None` ⇒ no bodies (the
    /// common, cheap case). The AABB is a generous superset of the view; the render pass culls
    /// precisely by projection, so a few extra layouts at the edge are harmless.
    pub fn build(sim: &Sim, tick: u64, body_near: Option<(Vec2, f32)>) -> Self {
        let creatures = sim
            .creatures
            .iter()
            .map(|c| {
                let pos = c.pos;
                let body = body_near.and_then(|(center, half)| {
                    ((pos.x - center.x).abs() < half && (pos.y - center.y).abs() < half)
                        .then(|| c.body_layout_for_render())
                });
                CreatureDot { id: c.id, pos, coloration: c.coloration(), biomass: c.biomass(), body }
            })
            .collect();
        RenderSnapshot { tick, creatures }
    }
}
