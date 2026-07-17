//! Input event collection and dispatch.
//! Extracted from main.rs to consolidate scattered key checks.

use macroquad::prelude::*;
use crate::ui::UiOut;

/// An input event from keyboard/input that affects app state.
#[derive(Debug, Clone, Copy)]
pub enum InputEvent {
    /// Space: toggle sim pause (only valid if sim is running)
    TogglePause,
    /// Right: step the sim once while paused (only valid if sim is running)
    StepOnce,
    /// T: toggle between hex and cube terrain rendering
    ToggleTerrainKind,
    /// U-3: N key — regenerate the world with a new seed (only valid in Procgen+standalone mode)
    RegenSeed,
    /// U-9: H key — toggle panel visibility (NOT gated by UI keyboard focus)
    ToggleUiVisibility,
}

/// Collect all input events this frame, respecting UI gating.
/// Returns a vec of input events detected from keyboard state.
/// When UI wants keyboard input, sim controls (Space/Right/N/T) are gated off.
/// H key (toggle UI visibility) is NEVER gated, even when UI has keyboard focus.
pub fn collect(ui_out: &UiOut) -> Vec<InputEvent> {
    let mut events = Vec::new();

    // U-9: H key is not gated — controls UI visibility itself
    if is_key_pressed(KeyCode::H) {
        events.push(InputEvent::ToggleUiVisibility);
    }

    // Gate sim/terrain controls when UI has keyboard focus
    if !ui_out.wants_keyboard {
        if is_key_pressed(KeyCode::Space) {
            events.push(InputEvent::TogglePause);
        }
        if is_key_pressed(KeyCode::Right) {
            events.push(InputEvent::StepOnce);
        }
        // U-3: N key for world reseed (gating to Procgen+standalone happens in main.rs)
        if is_key_pressed(KeyCode::N) {
            events.push(InputEvent::RegenSeed);
        }
        if is_key_pressed(KeyCode::T) {
            events.push(InputEvent::ToggleTerrainKind);
        }
    }

    events
}
