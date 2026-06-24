//! Input-stream infrastructure (R18, doc 12 §6.1). The Phase-0 stream is EMPTY — only the shape and
//! the tie-break ordering exist from M0, so the replay carrier (`seed + input log`) is wired before
//! any player agency arrives (Phase 1+).

/// What an input event does. Each event is tick-stamped (not wall-clock), so a different frame rate
/// never shifts the moment of application. The variant set widens with later phases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    PinEntity { id: u64 },
    UnpinEntity { id: u64 },
    SetParam { key: u64, val: i64 },
}

impl InputKind {
    /// Stable discriminant for the deterministic per-tick tie-break. Independent of memory layout.
    pub fn discriminant(&self) -> u8 {
        match self {
            InputKind::PinEntity { .. } => 0,
            InputKind::UnpinEntity { .. } => 1,
            InputKind::SetParam { .. } => 2,
        }
    }

    /// The secondary tie-break key (`id`, or `key` for params).
    pub fn id(&self) -> u64 {
        match self {
            InputKind::PinEntity { id } | InputKind::UnpinEntity { id } => *id,
            InputKind::SetParam { key, .. } => *key,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputEvent {
    pub tick: u64,
    pub kind: InputKind,
}

/// Canonical ordering of the events landing on one tick: `(discriminant(kind), id)` — NEVER arrival
/// order. The same `seed + input log` must reduce to the same run regardless of how events queued.
pub fn sort_tick_events(events: &mut [InputEvent]) {
    events.sort_by_key(|e| (e.kind.discriminant(), e.kind.id()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tie_break_is_kind_then_id() {
        let mut ev = vec![
            InputEvent { tick: 1, kind: InputKind::SetParam { key: 5, val: 0 } },
            InputEvent { tick: 1, kind: InputKind::PinEntity { id: 9 } },
            InputEvent { tick: 1, kind: InputKind::PinEntity { id: 3 } },
        ];
        sort_tick_events(&mut ev);
        assert_eq!(ev[0].kind, InputKind::PinEntity { id: 3 });
        assert_eq!(ev[1].kind, InputKind::PinEntity { id: 9 });
        assert_eq!(ev[2].kind, InputKind::SetParam { key: 5, val: 0 });
    }
}
