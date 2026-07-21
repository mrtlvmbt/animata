//! U-2: Unified world building pipeline (single build path for app + harnesses).
//!
//! `build_world(spec, on_stage)` is the ONLY entry point for creating a BuiltWorld.
//! Called synchronously by:
//! - Worker thread (app): on a dedicated thread, with progress reporting to LoadState
//! - Harnesses (tests): inline, with no-op callback
//!
//! This function encodes the six D5 rules (critic-hardened):
//! 1. Config re-derived from spec.seed (NOT stored in spec)
//! 2. Landform flags from (spec.seed, spec.standalone) via landform_flags()
//! 3. Dim rule: standalone honors dim_request, sim mode uses config.econ.world_dim
//! 4. Effective dim is OUTPUT (built.dim), not input
//! 5. Send discipline: Box<dyn WorldView + Send>, raw chunk buffers only
//! 6. Single build path: no sync-vs-async divergence

use crate::raw_chunk::{BuiltWorld, BuildError};
use crate::world_spec::{WorldSpec, WorldSource, Stage, landform_flags};
use crate::terrain::build_raw_hex_terrain;
use crate::terrain_cube::build_raw_cube_terrain;
use sim_core::WorldView;
use world::ProcgenWorld;

/// Build a complete world: terrain, meshes, metadata.
///
/// This is the single, unified build path for all contexts (app worker + harnesses).
/// The callback is called at stage boundaries; it receives Stage and can inject delays
/// (e.g., --slow-load). Callback returns true to continue, false to abort (not used yet).
/// `height_scale_override` is an optional multiplier for the height→prism mapping (default ×1.0).
///
/// Returns a Send-safe BuiltWorld with OUTPUT dim (not the input spec.dim_request).
pub fn build_world<F>(spec: &WorldSpec, mut on_stage: F, height_scale_override: Option<f32>, enable_plate_sim: bool) -> Result<BuiltWorld, BuildError>
where
    F: FnMut(Stage) -> bool,
{
    // U-11: Report heightfield generation start
    on_stage(Stage::GenerateHeightfield);

    // Step 1: Create config from spec.seed (never from parse_args — F18)
    let config = cli::default_config(spec.seed);

    // Step 2: Compute effective dim (D5 dim rule)
    let effective_dim = compute_effective_dim(&spec.source, spec.standalone, config.econ.world_dim);

    // Step 3: Create the world (ProcgenWorld or loaded dump)
    let world: Box<dyn WorldView + Send> = match &spec.source {
        WorldSource::Procgen { dim_request: _ } => {
            // U-10: Use explicit flags if provided; otherwise derive from seed (D5)
            let flags = spec.explicit_landform_flags.unwrap_or_else(|| landform_flags(spec.seed, spec.standalone));
            // U-11: No-op callback for world crate (observation-only, byte-pure)
            Box::new(ProcgenWorld::new_with_callback(
                effective_dim,
                cli::HMAX,
                cli::RESOURCE_BASE,
                config.seed ^ cli::WORLD_SALT,
                None,
                flags.base,
                flags.tect,
                flags.aeolian,
                flags.volcanic,
                flags.glacial,
                flags.coastal,
                flags.erosion,
                flags.ridges,
                flags.beaches,
                flags.erosion_strength,
                flags.glacial_strength,
                None, // U-11: No callback — progress reported via on_stage() below
                enable_plate_sim, // Slice-1e: threaded from CLI --plate-sim flag
                15, // plate_count
                100, // plate_strength
                60, // ela_threshold_percent
                -1, // sea_level (sentinel: -1 = unset)
                100, // volcanic_strength
                100, // aeolian_strength
                100, // coastal_strength
            ))
        }
        WorldSource::Dump(_) => {
            // Load v1 dump (carries its own dim; deferred: out of U-3 scope)
            // TODO: implement DumpWorld::load(path) to actually load the dump file
            // For now, fallback to Procgen (this makes --v1-dump flag non-functional as of U-3)
            eprintln!("[build_world] Dump loading not yet implemented; falling back to Procgen (--v1-dump flag ignored)");
            // U-10: Use explicit flags if provided; otherwise derive from seed
            let flags = spec.explicit_landform_flags.unwrap_or_else(|| landform_flags(spec.seed, spec.standalone));
            // U-11: No-op callback for world crate (observation-only, byte-pure)
            Box::new(ProcgenWorld::new_with_callback(
                effective_dim,
                cli::HMAX,
                cli::RESOURCE_BASE,
                config.seed ^ cli::WORLD_SALT,
                None,
                flags.base,
                flags.tect,
                flags.aeolian,
                flags.volcanic,
                flags.glacial,
                flags.coastal,
                flags.erosion,
                flags.ridges,
                flags.beaches,
                flags.erosion_strength,
                flags.glacial_strength,
                None, // U-11: No callback — progress reported via on_stage() below
                enable_plate_sim, // Slice-1e: threaded from CLI --plate-sim flag
                15, // plate_count
                100, // plate_strength
                60, // ela_threshold_percent
                -1, // sea_level (sentinel: -1 = unset)
                100, // volcanic_strength
                100, // aeolian_strength
                100, // coastal_strength
            ))
        }
    };

    // D4: Build meshes
    on_stage(Stage::BuildMeshes);

    // Step 4: Build hex and cube terrain (raw buffers; no GPU calls here)
    let hex = build_raw_hex_terrain(effective_dim, world.as_ref(), spec.seed, spec.bare_mode, height_scale_override)?;
    let cube = build_raw_cube_terrain(effective_dim, world.as_ref(), spec.seed, spec.bare_mode, height_scale_override)?;

    // D4: Done
    on_stage(Stage::Done);

    Ok(BuiltWorld {
        world,
        dim: effective_dim,  // OUTPUT dim, not input
        hex,
        cube,
        seed: spec.seed,
    })
}

/// Compute the effective world dimension (D5 dim rule).
///
/// Standalone mode: honors dim_request if provided, else uses config default.
/// Sim mode: always uses config.econ.world_dim (render dim must match sim's).
/// Slice-4a: Standalone default dim (256 for physics relief to read well).
const STANDALONE_DEFAULT_DIM: i64 = 256;

fn compute_effective_dim(source: &WorldSource, standalone: bool, config_dim: i64) -> i64 {
    match source {
        WorldSource::Procgen { dim_request } => {
            if standalone {
                // Slice-4a: Standalone default is 256 (physics relief); --dim N overrides.
                // Sim mode (standalone=false) stays on config_dim (64) per pinned-param contract.
                dim_request.unwrap_or(STANDALONE_DEFAULT_DIM)
            } else {
                // Sim mode: ignore dim_request (pinned-param contract)
                config_dim
            }
        }
        WorldSource::Dump(_) => {
            // Dump mode: dim comes from file (will be loaded in U-3)
            config_dim  // Fallback for now
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_effective_dim_standalone_with_request() {
        let source = WorldSource::Procgen { dim_request: Some(512) };
        let dim = compute_effective_dim(&source, true, 256);
        assert_eq!(dim, 512, "standalone should honor dim_request");
    }

    #[test]
    fn compute_effective_dim_standalone_without_request() {
        let source = WorldSource::Procgen { dim_request: None };
        let dim = compute_effective_dim(&source, true, 256);
        assert_eq!(dim, 256, "standalone should use config default when dim_request is None");
    }

    #[test]
    fn compute_effective_dim_sim_mode_ignores_request() {
        let source = WorldSource::Procgen { dim_request: Some(512) };
        let dim = compute_effective_dim(&source, false, 256);
        assert_eq!(dim, 256, "sim mode must ignore dim_request (pinned-param contract)");
    }
}
