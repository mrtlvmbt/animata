# U-3: In-game world reseed — implementation notes

## Part 1: Determinism fix (F14)

**Issue:** Previous coder left the code incomplete. Line 883 in main.rs tried to assign to `spec.seed`, but `spec` was declared as `let spec` (immutable).

**Fix:** Changed line 351 to `let mut spec` to allow the assignment on line 887.

**Regression testing:** Verified both rendering paths (default CPU mesh + GPU retained mesh) produce byte-identical output:
- Cold launch: `--seed 1 --dim 256 --cam iso-default --screenshot cold.png`
- Baseline: generated from commit ce616d9 (merge base of render-r12-terragen-preview and u3-reseed)
- Result: IDENTICAL on both default and --retained modes

## Part 2: RegenChipPanel (in-game progress indicator)

**Architecture:**
- New struct `RegenChipPanel` implements `Panel` trait (v2/crates/render/src/ui/mod.rs)
- Anchored to RightTop corner (16px offset)
- Draws dark-glass background chip with pulsing dot + stage text + progress bar
- Automatically hides when no regen is in flight (checks for `regen_load_state` in UiCtx)

**Integration:**
1. Added `regen_load_state: Option<&LoadState>` to UiCtx struct
2. Created LoadState for in-game reseed (similar to U-2 loader pattern)
3. Modified world_builder callback to report stage progress to LoadState
4. Registered RegenChipPanel in UiRoot alongside DebugPanel
5. Passed regen_load_state to UiCtx in main loop (line 803)

**Progress tracking:**
- Uses same LoadState atomics as U-2 loader (shared infrastructure)
- Worker thread updates stage via callback during build_world
- Chip displays stage name ("Generating world" / "Building meshes") + progress percent
- Pulsing dot animates at 1.6s period (same as loader modal)

**Manual verification:**
- App path (Running phase): Press N key to trigger regen → chip appears showing progress
- Regen completes → chip disappears automatically
- Non-modal: input stays live, old world renders beneath the chip during build

**Deferred:**
- Automated screenshot capture during chip display (would require significant refactoring of screenshot mode's blocking recv loop)
- Manual PM verification of chip visual appearance and legibility required
