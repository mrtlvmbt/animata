//! Rendering subsystem split out of `main.rs`: camera, GPU pipelines, mesh building, and
//! the LOD chunk streamer. (HUD text + the debug minimap + input handling stay inline in
//! `main.rs` for now.)

pub mod camera;
pub mod gpu;
pub mod mesh;
pub mod streamer;
