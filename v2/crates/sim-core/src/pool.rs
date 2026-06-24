//! The sim's OWN thread pool (F5): an explicit `N` from config, NOT `num_cpus` and NOT the global
//! bevy `TaskPool` default (whose width depends on the machine → the signal golden would diverge
//! between dev-arm64 and `macos-latest`). One test process builds pools with `N=1` and `N>1` to run
//! the R14 gate; `RUST_TEST_THREADS` (the test runner) is orthogonal to this pool.

use crate::MergeStrategy;
use bevy_ecs::prelude::Resource;
use std::sync::Arc;

/// The scatter thread pool, shared as an ECS resource. Rayon with an EXPLICIT thread count.
#[derive(Resource, Clone)]
pub struct SimPool(pub Arc<rayon::ThreadPool>);

impl SimPool {
    pub fn new(threads: usize) -> Self {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads.max(1))
            .build()
            .expect("build sim thread pool");
        SimPool(Arc::new(pool))
    }
}

/// Scatter knobs read by stage 8 (number of deposit batches to form + the merge strategy).
#[derive(Resource, Clone, Copy)]
pub struct ScatterParams {
    pub threads: usize,
    pub strategy: MergeStrategy,
}
