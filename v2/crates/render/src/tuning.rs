//! U-7: Tuning configuration — feel and key mapping overrides.
//!
//! Loads optional `render-tuning.toml` from the current working directory.
//! Allows swapping zoom sensitivity, pan speed, drag sensitivity, and key mappings
//! without recompilation.
//!
//! If the file is absent, uses compiled defaults.
//! If present but malformed, logs warnings and falls back to defaults for each field.

use macroquad::prelude::KeyCode;
use std::collections::HashMap;

/// Camera and input tuning parameters.
#[derive(Clone, Debug)]
pub struct Tuning {
    pub zoom_rate: f32,
    pub pan_speed: f32,
    pub drag_sensitivity: f32,
    pub key_pan_up: KeyCode,
    pub key_pan_down: KeyCode,
    pub key_pan_left: KeyCode,
    pub key_pan_right: KeyCode,
    pub key_rotate_ccw: KeyCode,
    pub key_rotate_cw: KeyCode,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            zoom_rate: 0.075,
            pan_speed: 20.0,
            drag_sensitivity: 0.2,
            key_pan_up: KeyCode::W,
            key_pan_down: KeyCode::S,
            key_pan_left: KeyCode::A,
            key_pan_right: KeyCode::D,
            key_rotate_ccw: KeyCode::Q,
            key_rotate_cw: KeyCode::E,
        }
    }
}

impl Tuning {
    /// Load tuning from `render-tuning.toml` in the current working directory.
    /// If the file doesn't exist, returns `Default`.
    /// If parsing fails, logs warnings and uses defaults for invalid fields.
    pub fn load() -> Self {
        let path = "render-tuning.toml";
        let default = Tuning::default();

        match std::fs::read_to_string(path) {
            Ok(content) => Self::parse_toml(&content, default),
            Err(_) => {
                // File not found or unreadable — silently use defaults
                default
            }
        }
    }

    /// Parse TOML content line-by-line (hand-rolled parser to avoid adding crate dependencies).
    /// Format: `key = value` (no sections, no nesting).
    /// Supported keys:
    /// - `zoom_rate: f32`
    /// - `pan_speed: f32`
    /// - `drag_sensitivity: f32`
    /// - `key_pan_up: KeyCode` (e.g., "W", "Up", "Comma")
    /// - `key_pan_down: KeyCode`
    /// - `key_pan_left: KeyCode`
    /// - `key_pan_right: KeyCode`
    /// - `key_rotate_ccw: KeyCode`
    /// - `key_rotate_cw: KeyCode`
    fn parse_toml(content: &str, mut result: Tuning) -> Self {
        for line in content.lines() {
            let line = line.trim();
            // Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim().trim_matches('"').trim();

                match key {
                    "zoom_rate" => {
                        if let Ok(v) = value.parse::<f32>() {
                            result.zoom_rate = v;
                        } else {
                            eprintln!("[render-tuning] warning: invalid zoom_rate '{}', using default", value);
                        }
                    }
                    "pan_speed" => {
                        if let Ok(v) = value.parse::<f32>() {
                            result.pan_speed = v;
                        } else {
                            eprintln!("[render-tuning] warning: invalid pan_speed '{}', using default", value);
                        }
                    }
                    "drag_sensitivity" => {
                        if let Ok(v) = value.parse::<f32>() {
                            result.drag_sensitivity = v;
                        } else {
                            eprintln!("[render-tuning] warning: invalid drag_sensitivity '{}', using default", value);
                        }
                    }
                    "key_pan_up" => {
                        if let Some(kc) = parse_keycode(value) {
                            result.key_pan_up = kc;
                        } else {
                            eprintln!("[render-tuning] warning: invalid key_pan_up '{}', using default", value);
                        }
                    }
                    "key_pan_down" => {
                        if let Some(kc) = parse_keycode(value) {
                            result.key_pan_down = kc;
                        } else {
                            eprintln!("[render-tuning] warning: invalid key_pan_down '{}', using default", value);
                        }
                    }
                    "key_pan_left" => {
                        if let Some(kc) = parse_keycode(value) {
                            result.key_pan_left = kc;
                        } else {
                            eprintln!("[render-tuning] warning: invalid key_pan_left '{}', using default", value);
                        }
                    }
                    "key_pan_right" => {
                        if let Some(kc) = parse_keycode(value) {
                            result.key_pan_right = kc;
                        } else {
                            eprintln!("[render-tuning] warning: invalid key_pan_right '{}', using default", value);
                        }
                    }
                    "key_rotate_ccw" => {
                        if let Some(kc) = parse_keycode(value) {
                            result.key_rotate_ccw = kc;
                        } else {
                            eprintln!("[render-tuning] warning: invalid key_rotate_ccw '{}', using default", value);
                        }
                    }
                    "key_rotate_cw" => {
                        if let Some(kc) = parse_keycode(value) {
                            result.key_rotate_cw = kc;
                        } else {
                            eprintln!("[render-tuning] warning: invalid key_rotate_cw '{}', using default", value);
                        }
                    }
                    _ => {
                        eprintln!("[render-tuning] warning: unknown key '{}', ignoring", key);
                    }
                }
            }
        }
        result
    }
}

/// Parse a key name string to a KeyCode.
/// Supports common names like "W", "A", "Up", "Left", "Comma", etc.
fn parse_keycode(name: &str) -> Option<KeyCode> {
    match name {
        "W" => Some(KeyCode::W),
        "A" => Some(KeyCode::A),
        "S" => Some(KeyCode::S),
        "D" => Some(KeyCode::D),
        "Q" => Some(KeyCode::Q),
        "E" => Some(KeyCode::E),
        "Up" => Some(KeyCode::Up),
        "Down" => Some(KeyCode::Down),
        "Left" => Some(KeyCode::Left),
        "Right" => Some(KeyCode::Right),
        "Comma" => Some(KeyCode::Comma),
        "Period" => Some(KeyCode::Period),
        "Space" => Some(KeyCode::Space),
        "Enter" => Some(KeyCode::Enter),
        "Escape" => Some(KeyCode::Escape),
        "Tab" => Some(KeyCode::Tab),
        "0" => Some(KeyCode::Key0),
        "1" => Some(KeyCode::Key1),
        "2" => Some(KeyCode::Key2),
        "3" => Some(KeyCode::Key3),
        "4" => Some(KeyCode::Key4),
        "5" => Some(KeyCode::Key5),
        "6" => Some(KeyCode::Key6),
        "7" => Some(KeyCode::Key7),
        "8" => Some(KeyCode::Key8),
        "9" => Some(KeyCode::Key9),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tuning_default() {
        let t = Tuning::default();
        assert_eq!(t.zoom_rate, 0.075);
        assert_eq!(t.pan_speed, 20.0);
        assert_eq!(t.key_pan_up, KeyCode::W);
        assert_eq!(t.key_rotate_ccw, KeyCode::Q);
    }

    #[test]
    fn test_parse_keycode() {
        assert_eq!(parse_keycode("W"), Some(KeyCode::W));
        assert_eq!(parse_keycode("Q"), Some(KeyCode::Q));
        assert_eq!(parse_keycode("Comma"), Some(KeyCode::Comma));
        assert_eq!(parse_keycode("Invalid"), None);
    }

    #[test]
    fn test_tuning_parse() {
        let toml = "zoom_rate = 0.05\npan_speed = 25.0\nkey_pan_up = E";
        let t = Tuning::parse_toml(toml, Tuning::default());
        assert_eq!(t.zoom_rate, 0.05);
        assert_eq!(t.pan_speed, 25.0);
        assert_eq!(t.key_pan_up, KeyCode::E);
        // Others should remain default
        assert_eq!(t.key_pan_down, KeyCode::S);
    }

    #[test]
    fn test_tuning_parse_with_comments() {
        let toml = "# Comment\nzoom_rate = 0.06\n# Another comment\npan_speed = 30.0";
        let t = Tuning::parse_toml(toml, Tuning::default());
        assert_eq!(t.zoom_rate, 0.06);
        assert_eq!(t.pan_speed, 30.0);
    }
}
