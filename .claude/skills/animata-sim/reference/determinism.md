# Determinism — the contract, in depth

**Read this when** you touch anything that could move the golden: the checksum, RNG, a parallel phase,
or persist. The one rule: **the sim replays BIT-IDENTICALLY within a profile.** `SKILL.md` §0 is the
spine; this is the mechanism behind it.

## The golden lock

`state_checksum(&Sim, &VoxelTerrain) -> u64` (`sim.rs:1523`) folds the FULL state by a **hand-written
field list** (it is NOT derived) — every `Creature` field + every genome `Vec<f32>` (`Genome::checksum`
in `genome.rs`) + all mutable terrain (`mut_state_checksum` in `terrain.rs`). A 300-tick run on seed 42
must equal `GOLDEN_CHECKSUM_SEED42_300` (`sim.rs:1560`); the long lock is `GOLDEN_CHECKSUM_SEED1_8000`
(`sim.rs:1573`). Because the fold is hand-written, a NEW field silently escapes the lock unless you add
it — see the §7 checklist in `SKILL.md`.

Re-pin procedure, "did I MEAN to move it?", and the inert-vs-trajectory distinction live in `SKILL.md`
§3/§6 — the canonical source. Don't duplicate them; follow them.

## profiles: debug ≠ release (FMA)

The two profiles diverge over thousands of ticks because release contracts `a*b+c` into a single fused
`fma` (more precision) while debug does not — so the golden is pinned PER PROFILE via
`cfg!(debug_assertions)` (see both arms at `sim.rs:1560`). **Canonical verification profile = release**
(acceptance corridors are tuned there). Mechanism & sources: `external-references.md` §1.

## RNG discipline

- Only `seed_fold(world_seed, &[SALT, id, tick])` (`rng.rs:18`) or `splitmix64` (`rng.rs:8`) on
  `(id, tick)`. Never wall-clock, never iteration-order-dependent.
- A per-pair roll (predator i × prey j × tick) STAYS a per-pair roll — don't collapse it to per-creature.
- **Never float-add into a hash or determinism-critical aggregate.** Float `+` is not associative
  (`external-references.md` §2), so a rayon reorder gives different bits. Use `f32::to_bits` + an integer
  fold (`fnv_fold_u32`/`fnv_fold_u64`, `rng.rs:50`/`:41`).
- Independent streams come from distinct `SALT` constants XOR-mixed through splitmix
  (`external-references.md` §6). Toggling one feature's stream must not perturb another's draw sequence.

## parallel decide / serial apply — the crux

Decide is parallel because it is READ-ONLY (writes `decisions[i]` per index, collected in index order).
Apply parallelises only the per-creature `map` that emits an `Outcome` (each creature mutates only
itself); the **mutation replay is SERIAL in fixed index order**. Any float aggregate on the critical
path is integer or a serial fixed-order sum.

**The one in-tick read-after-write is OXYGEN** (`deposit_oxygen` writes `oxygen[i] += amt`, `oxygen_at`
`terrain.rs:1104` reads it the same tick). Nutrient is written but never read back in apply, so buffering
it in index order is byte-identical. Parallel apply therefore needs exactly ONE semantic choice: read
START-OF-TICK oxygen (immutable pre-apply terrain). That moved the golden once, by design (PR #85). See
memory `sim-perf-100k-scaleup` for which seeds actually shifted.

## persist (`persist.rs`)

`bincode` is **positional / no-schema**: a field added or shifted renders every following byte, so a
pre-change snapshot must be *rejected*, not mis-decoded. Discipline (`external-references.md` §3):

- Current file MAGIC = ASCII **"ANM4"** = `0x414E_4D34` (`persist.rs:31` — the authoritative value; an
  older inline comment naming "ANM3" is stale, trust the const). `MAGIC_V2` ("ANM2") decodes through the
  frozen `persist_v2.rs` shapes via `v2::migrate`. To add a version: bump `MAGIC`, FREEZE the prior shape,
  add a migrate arm.
- **The serde wire-proxy trick (Tier-3, PR #92):** `TerrainState` carries
  `#[serde(into = "TerrainStateWire", from = "TerrainStateWire")]` (`terrain.rs:391`). The in-RAM layout
  is the packed `BioCell`, but it (de)serializes THROUGH the old 3-separate-Vec `TerrainStateWire`
  (`terrain.rs:417`) → disk bytes stay identical to ANM4, **MAGIC does NOT bump**, old saves decode
  unchanged. A RAM relayout with zero persist churn. See `terrain.md`.
- **Padding never enters disk or the hash.** A packed `#[repr]` cell has indeterminate padding bytes —
  fold and (de)serialize **field-by-field explicitly**, in the same order as before, never as raw cell
  bytes (`mut_state_checksum` does this).

## When the golden moves unexpectedly

It is a **bug**, not a re-pin trigger — a parallel reduce that reordered a float sum, a new field folded
in non-deterministic order, a per-pair roll collapsed. Find the leak (`SKILL.md` §3/§5). Re-pin ONLY for
an intended trajectory change, and then do the §5 multi-seed corridor work — see
`corridors-and-fragility.md`.
