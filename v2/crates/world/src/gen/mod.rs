//! World-gen pipeline stage home (PM plan-consensus, W-ladder). Every stage (W-1 height, W-2
//! climate/biome, W-3 hydrology, W-4 erosion, W-5 resource-caps, W-6 assembly) lives under this
//! module, one file/sub-module per stage. **Every `.rs` under `world/src/gen/` is covered by the
//! recursive glob no-float guard** (`world/tests/no_float_guard_gen.rs`, pattern
//! `world/src/gen/**/*.rs`) — a stage file needs NO per-file registration to be scanned, unlike
//! `sim-core`'s hardcoded module allow-list (`no_float_guard.rs`). This is deliberate: the whole
//! pipeline must be pure integer/fixed-point (the point of replacing the legacy `NoiseWorld`'s
//! `f64 sin` path), and the glob makes that enforced-by-construction rather than resting on
//! remembering to add each new file to a list.
//!
//! **W-6 status**: `gen::caps::classify_and_caps` now backs the production `ProcgenWorld`
//! (`world/src/lib.rs`) — the pipeline is WIRED. `NoiseWorld`, the last `f64 sin` in the world
//! path, is deleted.

pub mod biome;
pub mod caps;
pub mod climate;
pub mod drainage;
pub mod erosion;
pub mod height;
pub mod material;
pub mod moisture;
