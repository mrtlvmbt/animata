//! `DetMap` — the deterministic-iteration map for core state.
//!
//! Bare `std::collections::HashMap` is BANNED in core state: its random hasher (OS-seeded) makes
//! iteration order vary run-to-run, and any such order that feeds the tick or the state hash leaks
//! non-determinism. `BTreeMap` always iterates in key order. The zero-float / no-HashMap guard test
//! enforces this mechanically.
pub type DetMap<K, V> = std::collections::BTreeMap<K, V>;
