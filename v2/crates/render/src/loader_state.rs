//! U-2: App phase and loading state management.
//!
//! Tracks whether the app is in Loading or Running phase, and communicates
//! world-build progress from worker thread to main thread via Arc<Atomic*> fields.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;
use crate::world_spec::Stage;

/// Shared state for loading progress (worker thread → main thread via atomic updates).
///
/// All fields are Arc-wrapped atomic types: lock-free, wait-free updates.
/// Main thread polls these to drive the loading UI; no synchronous locks.
#[derive(Clone)]
pub struct LoadState {
    /// Progress as permille (0–1000). Updated by worker via on_stage callback.
    pub progress: Arc<AtomicU32>,
    /// Current stage (0=GenerateWorld, 1=BuildMeshes, 2=Done). Drives UI checklist.
    pub step: Arc<AtomicU8>,
    /// Seed (for minimap cache keys, etc.).
    pub seed: u64,
    /// True when build completes (set by worker before finishing).
    pub is_done: Arc<AtomicBool>,
}

impl LoadState {
    /// Create a new load state for the given seed.
    pub fn new(seed: u64) -> Self {
        LoadState {
            progress: Arc::new(AtomicU32::new(0)),
            step: Arc::new(AtomicU8::new(0)),
            seed,
            is_done: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Set progress (permille: 0–1000). Safe to call from any thread.
    pub fn set_progress(&self, permille: u32) {
        self.progress.store(permille.clamp(0, 1000), Ordering::Release);
    }

    /// Set current stage. Safe to call from any thread.
    pub fn set_stage(&self, stage: Stage) {
        self.step.store(stage.as_u8(), Ordering::Release);
    }

    /// Mark build as done. Safe to call from any thread.
    pub fn mark_done(&self) {
        self.is_done.store(true, Ordering::Release);
    }

    /// Get current progress (permille).
    pub fn get_progress(&self) -> u32 {
        self.progress.load(Ordering::Acquire)
    }

    /// Get current stage.
    pub fn get_stage(&self) -> Stage {
        let v = self.step.load(Ordering::Acquire);
        Stage::from_u8(v).unwrap_or(Stage::GenerateWorld)
    }

    /// Check if build is complete.
    pub fn is_complete(&self) -> bool {
        self.is_done.load(Ordering::Acquire)
    }
}

/// App execution phase: Loading or Running.
///
/// Determines what gets rendered each frame:
/// - Loading: shows the loader modal only (no world, camera, etc.).
/// - Running: shows the world (terrain, creatures, HUD).
#[derive(Clone)]
pub enum AppPhase {
    /// World is building; show loader modal and wait for worker completion.
    Loading(LoadState),
    /// World is complete; render terrain, creatures, HUD.
    Running,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_state_initial_state() {
        let ls = LoadState::new(0x1234_5678);
        assert_eq!(ls.get_progress(), 0);
        assert_eq!(ls.get_stage(), Stage::GenerateWorld);
        assert!(!ls.is_complete());
        assert_eq!(ls.seed, 0x1234_5678);
    }

    #[test]
    fn load_state_progress_updates() {
        let ls = LoadState::new(42);
        ls.set_progress(500);
        assert_eq!(ls.get_progress(), 500);
        ls.set_progress(1000);
        assert_eq!(ls.get_progress(), 1000);
        // Clamp overflow
        ls.set_progress(2000);
        assert_eq!(ls.get_progress(), 1000);
    }

    #[test]
    fn load_state_stage_updates() {
        let ls = LoadState::new(42);
        ls.set_stage(Stage::GenerateWorld);
        assert_eq!(ls.get_stage(), Stage::GenerateWorld);
        ls.set_stage(Stage::BuildMeshes);
        assert_eq!(ls.get_stage(), Stage::BuildMeshes);
        ls.set_stage(Stage::Done);
        assert_eq!(ls.get_stage(), Stage::Done);
    }

    #[test]
    fn load_state_done_flag() {
        let ls = LoadState::new(42);
        assert!(!ls.is_complete());
        ls.mark_done();
        assert!(ls.is_complete());
    }
}
