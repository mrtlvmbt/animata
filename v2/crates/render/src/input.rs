//! Input event collection and dispatch.
//! Extracted from main.rs to consolidate scattered key checks.

use macroquad::prelude::*;

/// An input event from keyboard/input that affects app state.
#[derive(Debug, Clone, Copy)]
pub enum InputEvent {
    /// Space: toggle sim pause (only valid if sim is running)
    TogglePause,
    /// Right or N: step the sim once while paused (only valid if sim is running)
    StepOnce,
    /// T: toggle between hex and cube terrain rendering
    ToggleTerrainKind,
    /// C: toggle between Height and Material coloring (rebuilds terrain meshes)
    ToggleColorMode,
}

/// Collect all input events this frame.
/// Returns a vec of input events detected from keyboard state.
pub fn collect() -> Vec<InputEvent> {
    let mut events = Vec::new();

    if is_key_pressed(KeyCode::Space) {
        events.push(InputEvent::TogglePause);
    }
    if is_key_pressed(KeyCode::Right) || is_key_pressed(KeyCode::N) {
        events.push(InputEvent::StepOnce);
    }
    if is_key_pressed(KeyCode::T) {
        events.push(InputEvent::ToggleTerrainKind);
    }
    if is_key_pressed(KeyCode::C) {
        events.push(InputEvent::ToggleColorMode);
    }

    events
}
