//! World-gen pipeline stage home (PM plan-consensus, W-ladder). Every stage (W-1 height, W-2
//! climate/biome, W-3 hydrology, W-4 erosion, W-5 resource-caps, W-6 assembly) lives under this
//! module, one file/sub-module per stage. **Every `.rs` under `world/src/gen/` is covered by the
//! recursive glob no-float guard** (`world/tests/no_float_guard_gen.rs`, pattern
//! `world/src/gen/**/*.rs`) — a stage file needs NO per-file registration to be scanned, unlike
//! `sim-core`'s hardcoded module allow-list (`no_float_guard.rs`). This is deliberate: the whole
//! pipeline must be pure integer/fixed-point (the point of replacing `NoiseWorld`'s `f64 sin`
//! path), and the glob makes that enforced-by-construction rather than resting on remembering to
//! add each new file to a list.
//!
//! **W-1/W-2/W-3 status**: this module is compiled and tested but PROD-INERT — no `WorldView` impl
//! or `build_sim` calls into it yet (that wiring is W-6, golden-TOUCHING). The legacy `NoiseWorld`
//! (`world/src/lib.rs`, still `f64 sin`) sits OUTSIDE `gen/` and is deliberately NOT scanned by
//! the glob guard — it is deleted at W-6.

pub mod biome;
pub mod climate;
pub mod drainage;
pub mod height;
pub mod material;
pub mod moisture;
