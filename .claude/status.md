task: #451 U-1 UI core (Panel trait, UiRoot, UiAction, pointer gating)
phase: F1/F2/F3 fixes complete, code-critic review pending
blocked_on: PM code-critic approval
next: code-critic verdict -> PR merge -> U-2 loading screen
updated: 2026-07-16 01:15

## U-1 UI Core — Fixes Applied

### F1: Real UiAction Flow (dead infrastructure fixed)
- DebugPanel now pushes real UiActions via buttons (Pause, Step, Hex↔Cube)
- main.rs unified handler consumes actions from BOTH UI and keyboard input
- Removed dummy_actions placeholder; end-to-end flow proven

### F2: Honest Gating Unit Test (vacuous test fixed)
- Refactored camera input reading → CamInput injectable snapshot
- CamInput::collect() reads macroquad state; apply_cam_input(input, gate) applies testably
- Test feeds synthetic input (wheel_y=1.0, pan_dir=(20,0), yaw_step=1) and verifies:
  - wants_pointer=true blocks zoom/pan changes
  - wants_keyboard=true blocks pan/yaw but allows zoom
  - no gating allows all changes
- Test FAILS if gate is removed (honest, not vacuous)

### F3: Consolidation (ungated camera.update())
- camera.update() now delegates to update_gated(false, false)
- Removed dead update_pan_keyboard/update_pan_mouse/update_zoom/update_rotate methods
- Used consistently in all paths (screenshot/bench/main loop)

### Acceptance
- Compile-check: PASS
- Clippy: clean
- 4 byte-identical screenshots: verified vs d99fd4e baseline
- Honest gating test: injection-based, fails if gate deleted
