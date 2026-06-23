# Architecture — crate boundary, module map, the tick pipeline

**Read this when** you need the crate boundary, where a subsystem lives, or the order of the `step` phases.

## Two crates, compiler-enforced boundary

- **`crates/animata-sim`** (lib) — the deterministic sim. No macroquad; uses `glam`. All sim logic,
  the golden, persist. `bin/headless.rs` runs it without graphics.
- **`crates/animata`** (bin) — render (`main.rs` + `render/`), UI, `dev_bridge.rs` (behind `--features
  dev`). Reads the sim through a thin seam; owns no sim logic.

Keep sim logic in `animata-sim`, rendering/IO in `animata`. The split is what lets the sim be replayed
and checksummed without a GPU.

## Module map (`crates/animata-sim/src/`)

| File | Owns |
|------|------|
| `sim.rs` | `Sim`, `Creature`, `step` (`sim.rs:729`), `state_checksum` (`sim.rs:1523`), the goldens, metrics, the bench harness (`bench_populate` `sim.rs:640`). |
| `genome.rs` | GRN development (`develop`, `grow`), `Genome::checksum`. |
| `terrain.rs` | `TerrainGeo` (immutable `Arc`) + `TerrainState` (mutable); the getters; → see `terrain.md`. |
| `grid.rs` | `SpatialGrid` (neighbour queries, the predator-skip ring scan). |
| `rng.rs` | `splitmix64` (`rng.rs:8`), `seed_fold` (`rng.rs:18`), the FNV folds (`rng.rs:41`,`:50`). |
| `fastmath.rs` | deterministic fast-approx `tanh`/`exp`/`kleiber075` (`fastmath.rs:25`,`:38`,`:67`). |
| `config.rs` / `sim_config.rs` | tunable consts. |
| `persist.rs` / `persist_v2.rs` | save/load, MAGIC, the ANM2→current migration; → see `determinism.md` §persist. |
| `tectonics.rs` / `hydrology.rs` / `erosion.rs` / `lem.rs` | worldgen (one-time LOAD cost, NOT per-tick). |
| `pressure/` / `metrics/` | the pressure & metric registries (`eval(env,pheno,genome,ctx)->Effect`). |
| `clock.rs` | seasonality / tick clock. |

## The `step` pipeline (`sim.rs:729`) — phase order IS the determinism design

The tick is multi-phase precisely so the result is independent of iteration order. Phases (profiler
`Span::*` labels in `sim.rs`):

1. **Snapshot** (`Span::Snapshot`) — ONE parallel read-only pass builds the per-creature SoA columns
   (pos, biomass, carnivory, coloration, …) the later phases index. Terrain unmutated.
2. **GridRebuild** (`Span::GridRebuild`) + per-cell `pred_count` for the predator-skip gate.
3. **Decide** (`Span::Decide`) — parallel, READ-ONLY. Every creature `sense`s (plant field + nearest
   prey/threat) and the brain decides; writes `decisions[i]` per index. Non-predators skip the threat
   ring-scan when `pred_count` in reach is 0 (PR #82 gate).
4. **Predation** (`Span::Predation`) — resolve hunts by snapshot index, flag eaten prey dead.
5. **Apply** (`Span::Apply`) — parallel per-survivor `map` emits an `Outcome` (each creature mutates only
   itself), then a SERIAL index-order replay applies terrain deposits + death tally + birth queue. The
   one in-tick read-after-write is OXYGEN → see `determinism.md`.
6. **Develop** (`Span::Develop`) — `develop()` the queued births (per-birth, not per-tick).
7. **Compact** (`Span::Compact`) — flag-then-compact dead out, append births, cull to cap
   deterministically (random cull, never tail-truncation).

**Parallel only the read-only phases (snapshot/decide) + the per-creature apply map; all mutation is
serial in fixed index order.** That invariant is the whole reason replay is bit-exact — see
`determinism.md`.

## The development model (GRN → body) in brief

A genome is a gene-regulatory network, not a body blueprint. `develop()` grows a body from one seed cell
over `DEV_STEPS` (`regulate`: `s' = tanh(W·s + b)`, divide when a gene clears a threshold, capped at
`MAX_CELLS`). Develop-time → frozen to INTEGER `Phenotype` stats; the hot path reads only those. **Never
store coordinates on `Phenotype`** (kills `Copy`). Full detail: `SKILL.md` §2 (the canonical source) +
`genome.rs`.
