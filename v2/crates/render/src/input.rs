//! Input event collection and dispatch.
//! Extracted from main.rs to consolidate scattered key checks.

use macroquad::prelude::*;
use crate::ui::UiOut;

/// An input event from keyboard/input that affects app state.
#[derive(Debug, Clone, Copy)]
pub enum InputEvent {
    /// Space: toggle sim pause (only valid if sim is running)
    TogglePause,
    /// Right or N: step the sim once while paused (only valid if sim is running)
    StepOnce,
    /// T: toggle between hex and cube terrain rendering
    ToggleTerrainKind,
}

/// Collect all input events this frame, respecting UI gating.
/// Returns a vec of input events detected from keyboard state.
/// When UI wants keyboard input, sim controls (Space/Right/N/T) are gated off.
pub fn collect(ui_out: &UiOut) -> Vec<InputEvent> {
    let mut events = Vec::new();

    // Gate sim/terrain controls when UI has keyboard focus
    if !ui_out.wants_keyboard {
        if is_key_pressed(KeyCode::Space) {
            events.push(InputEvent::TogglePause);
        }
        if is_key_pressed(KeyCode::Right) || is_key_pressed(KeyCode::N) {
            events.push(InputEvent::StepOnce);
        }
        if is_key_pressed(KeyCode::T) {
            events.push(InputEvent::ToggleTerrainKind);
        }
    }

    events
}
