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
use animata_sim::terrain::VoxelTerrain;
use macroquad::math::{Vec2, Vec3};

use crate::ui::{CreatureView, LifeStats};

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

/// Inspector data for the selected creature, derived from its genome on the sim side. World-space
/// positions (not screen) so the renderer projects them with the current camera.
pub struct InspectView {
    pub view: CreatureView,
    /// World anchor of the selected creature (for the crosshair); `None` if off-column.
    pub world: Option<Vec3>,
    /// World anchors of same-species creatures (conspecific ring markers), capped.
    pub conspecific_world: Vec<Vec3>,
}

/// Everything the renderer + HUD read from the sim each frame, instead of touching `Sim`/`terrain`.
/// Built once per frame; in Phase B it is produced on the sim thread and read via an arc-swap.
pub struct RenderSnapshot {
    pub tick: u64,
    pub creatures: Vec<CreatureDot>,
    /// Population/evolution stats for the HUD; `None` until the world is ready.
    pub life: Option<LifeStats>,
    /// Selected-creature inspector bundle; `None` when nothing is selected / it died.
    pub inspect: Option<InspectView>,
    /// Per-phase `Sim::step` timing (label, mean ms) for the perf panel.
    pub sim_phases: Vec<(&'static str, f32)>,
    /// Live Amdahl split `(serial_ms, parallel_ms, serial_fraction)` — the speedup ceiling from more
    /// cores is `1 / serial_fraction`.
    pub amdahl: (f32, f32, f32),
}

impl RenderSnapshot {
    /// Build from the live sim + terrain. `selected` is the inspector's creature id (`None` = none).
    /// `body_near = Some((center_xz, half_extent))` requests body layouts for creatures within that
    /// world-space AABB (the high-zoom morphology LOD); `None` ⇒ no bodies (the common, cheap case).
    /// The AABB is a generous superset of the view; the render pass culls precisely by projection.
    pub fn build(
        sim: &Sim,
        terrain: &VoxelTerrain,
        tick: u64,
        selected: Option<u64>,
        body_near: Option<(Vec2, f32)>,
    ) -> Self {
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

        let (multi, _) = sim.complexity_mix();
        let life = Some(LifeStats {
            population: sim.population() as u64,
            avg_energy: sim.avg_energy(),
            avg_biomass: sim.avg_biomass(),
            multi,
            trophic: sim.trophic_fractions(),
            species: sim.species_count() as u64,
            niches: sim.niche_coverage(terrain) as u64,
            allop: sim.thermal_correlation(terrain),
            crypsis: sim.crypsis_correlation(terrain),
            strata: sim.stratum_mix(terrain),
        });

        // Inspector bundle for the selected creature (world-space; the renderer projects it). `None`
        // if the creature died — the main loop then clears the selection.
        let inspect = selected.and_then(|id| {
            let c = sim.creatures.iter().find(|c| c.id == id)?;
            let mut conspecific_world = Vec::new();
            for idx in sim.conspecifics(id) {
                if conspecific_world.len() >= 256 {
                    break;
                }
                conspecific_world.push(crate::creature_world(&sim.creatures[idx], terrain));
            }
            Some(InspectView {
                view: crate::creature_view(c, terrain),
                world: Some(crate::creature_world(c, terrain)),
                conspecific_world,
            })
        });

        let sim_phases =
            sim.profile_report().into_iter().map(|(span, mean, _max)| (span.label(), mean)).collect();

        RenderSnapshot { tick, creatures, life, inspect, sim_phases, amdahl: sim.profile_amdahl() }
    }
}
