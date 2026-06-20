use super::*;

/// `set` accepts every known name and rejects others; `pairs` reflects the change.
#[test]
fn features_set_and_pairs_round_trip() {
    let mut f = Features::default();
    assert!(f.pairs().iter().all(|(_, on)| *on), "defaults should be all-on");
    assert!(f.set("predation", false));
    assert!(!f.set("bogus", true), "unknown feature must be rejected");
    let on = f.pairs().iter().find(|(k, _)| *k == "predation").unwrap().1;
    assert!(!on, "pairs must reflect the set");
}

#[test]
fn params_set_and_pairs_round_trip() {
    let mut p = Params::default();
    assert!(p.set("thermal_penalty", 0.5));
    assert!(!p.set("nope", 1.0), "unknown param must be rejected");
    let v = p.pairs().iter().find(|(k, _)| *k == "thermal_penalty").unwrap().1;
    assert_eq!(v, 0.5);
}

/// A partial RON file overrides only what it names; everything else stays at the default.
#[test]
fn from_ron_partial_falls_back_to_defaults() {
    let cfg = SimConfig::from_ron("(features: (predation: false), params: (photo_rate: 5.0))").unwrap();
    assert!(!cfg.features.predation, "predation override not applied");
    assert!(cfg.features.climate, "unspecified feature must stay default-on");
    assert_eq!(cfg.params.photo_rate, 5.0);
    assert_eq!(cfg.params.thermal_penalty, Params::default().thermal_penalty, "unspecified param must stay default");
}

/// An empty RON document yields the full default config.
#[test]
fn from_ron_empty_is_default() {
    let cfg = SimConfig::from_ron("()").unwrap();
    assert!(cfg.features.pairs().iter().all(|(_, on)| *on));
    assert_eq!(cfg.params.thermal_penalty, Params::default().thermal_penalty);
}

/// Defaults equal the `config.rs` constants (the golden config).
#[test]
fn defaults_match_constants() {
    let p = Params::default();
    assert_eq!(p.thermal_penalty, THERMAL_PENALTY);
    assert_eq!(p.photo_rate, PHOTO_RATE);
    assert_eq!(p.camo_base_detect, CAMO_BASE_DETECT);
}
