//! R-1 snapshot-driver (port of v1 `sim_driver.rs`): owns the `Sim` on a WORKER thread, steps it at a
//! fixed cadence, and publishes a read-only [`RenderSnapshot`] to the render/main thread through a
//! double buffer.
//!
//! **Ownership (critic F6)**: the worker thread OWNS the `Sim` — it is the only thing that ever calls
//! `Sim::step`/`observe_render`. The render thread only ever reads a published, OWNED `RenderSnapshot`
//! (no borrow into the `Sim` — `observe_render`'s contract). This ownership split, not the swap
//! mechanism below, is what makes the cross-thread read sound; the render thread NEVER touches `Sim`.
//!
//! **Publish mechanism (critic F2)**: `ArcSwapOption` is wait-free and cannot poison. The v1
//! `sim_driver.rs` reference used `Arc<Mutex<Option<Arc<RenderSnapshot>>>>` — a panic while a
//! render-thread reader held that lock would poison it, and the worker's next `.lock().unwrap()`
//! would itself panic, taking the sim down with an unrelated render bug. `ArcSwapOption` has no lock
//! to poison, so a render-thread panic can never propagate to the worker.
//!
//! **Staleness (critic F4)**: the render thread may read a snapshot up to one tick behind the worker
//! (the worker can publish a newer one between the render thread's read and its use of it). Acceptable
//! for this proof-of-life; not a correctness issue since the render thread never mutates `Sim`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use arc_swap::ArcSwapOption;
use sim_core::{RenderSnapshot, Sim};

/// Fixed sim cadence for the proof-of-life driver. A real time-scale UI is R-6; R-1 only needs
/// pause/step to prove the seam.
const TICK_HZ: u64 = 30;
const TICK_INTERVAL: Duration = Duration::from_millis(1000 / TICK_HZ);

/// Handle held by the render/main thread. Cloning is cheap (all fields are `Arc`s) — not needed for
/// this single-window proof-of-life, but keeps the seam ready for a future multi-view HUD.
pub struct SimHandle {
    snapshot: Arc<ArcSwapOption<RenderSnapshot>>,
    paused: Arc<AtomicBool>,
    step_once: Arc<AtomicBool>,
    _thread: JoinHandle<()>,
}

impl SimHandle {
    /// The latest published snapshot, or `None` before the worker's first step completes.
    pub fn latest(&self) -> Option<Arc<RenderSnapshot>> {
        self.snapshot.load_full()
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn toggle_pause(&self) {
        self.paused.fetch_xor(true, Ordering::Relaxed);
    }

    /// Advance exactly one tick even while paused (single-step control).
    pub fn step_once(&self) {
        self.step_once.store(true, Ordering::Relaxed);
    }
}

/// Spawn the sim worker for `seed` (the `cli::default_config` economy — the same one every other v2
/// headless/test entry point uses). Publishes the tick-0 snapshot before returning so the HUD has
/// something to show on the very first render frame.
pub fn spawn(seed: u64) -> SimHandle {
    let snapshot: Arc<ArcSwapOption<RenderSnapshot>> = Arc::new(ArcSwapOption::from(None));
    let paused = Arc::new(AtomicBool::new(false));
    let step_once = Arc::new(AtomicBool::new(false));

    let snap_w = snapshot.clone();
    let paused_w = paused.clone();
    let step_once_w = step_once.clone();

    let thread = std::thread::Builder::new()
        .name("animata-v2-sim".into())
        .spawn(move || worker(seed, snap_w, paused_w, step_once_w))
        .expect("spawn v2 sim worker thread");

    SimHandle { snapshot, paused, step_once, _thread: thread }
}

fn worker(
    seed: u64,
    snapshot: Arc<ArcSwapOption<RenderSnapshot>>,
    paused: Arc<AtomicBool>,
    step_once: Arc<AtomicBool>,
) {
    // The worker OWNS `sim` for its entire lifetime (F6) — built exactly like every other v2 entry
    // point (`cli::build_sim`), never exposed to the render thread.
    let mut sim: Sim = cli::build_sim(cli::default_config(seed));
    publish(&snapshot, &sim);

    loop {
        let should_step = !paused.load(Ordering::Relaxed) || step_once.swap(false, Ordering::Relaxed);
        if should_step {
            sim.step();
            publish(&snapshot, &sim);
        }
        // Fixed cadence, paused or not — never busy-spins (critic: "no busy-spin").
        std::thread::sleep(TICK_INTERVAL);
    }
}

/// `observe_render` is `&self`/read-only by construction (the firewall the `cli` crate's
/// `v2_observe_render_is_golden_neutral` test pins) — calling it here can never perturb `sim`.
fn publish(snapshot: &ArcSwapOption<RenderSnapshot>, sim: &Sim) {
    snapshot.store(Some(Arc::new(sim.observe_render())));
}
