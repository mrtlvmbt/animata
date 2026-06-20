//! `animata-sim` — the simulation + world model, fully decoupled from graphics.
//!
//! This crate has NO dependency on macroquad (only `glam` for vector math, the same major
//! version macroquad re-exports, so `Vec2` is the identical type in the renderer). It is the
//! deterministic core: terrain generation, the developmental genome, the creature ecosystem and
//! its selection pressures, the world clock, and the seeded RNG / determinism checksum. It runs
//! headless (see `src/bin/headless.rs`) and under `cargo test` without any window or GL context.

pub mod clock;
pub mod config;
pub mod erosion;
pub mod genome;
pub mod grid;
pub mod hydrology;
pub mod metrics;
pub mod persist;
pub mod pressure;
pub mod rng;
pub mod sim;
pub mod sim_config;
pub mod tectonics;
pub mod terrain;
