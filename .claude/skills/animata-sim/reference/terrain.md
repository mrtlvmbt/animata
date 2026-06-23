# Terrain — the immutable/mutable split, the hot getters, packing

**Read this when** you touch terrain layout, the biomass/oxygen/nutrient getters, or worldgen.

## The split: immutable geo (Arc) vs mutable state

- **`TerrainGeo`** (`terrain.rs:364`) — per-column geometry + climate, generated ONCE at worldgen,
  shared by `Arc`, never mutated. Fields: `surf`, `flags`, `water`, `temp`, `bio_geo` (packed
  biome+moist, see below), `toxicity`, `water_dist`. The renderer meshes from this.
- **`TerrainState`** (`terrain.rs:392`) — the ONLY sim-mutated terrain: `bio` (packed biomass/nutrient,
  see below), `nutrient_update`, `oxygen` (f32), `oxygen_update`. This is what the checksum folds and
  what a save snapshot carries (the geo is regenerated from `seed`).

Packing **across** this boundary would break the Arc-shared-immutable design — so the Tier-3 work packed
*within* each side only.

## Tier-3 cache packing (PR #92, byte-identical)

The hot `sense` read (`current_biomass`→`cap_at`) needed 5 fields at one column index from 5 separate
arrays = ~5 scattered cache lines/sample. Co-read fields were packed:

- **`GeoCell { biome, moist }`** (`terrain.rs:341`) → `TerrainGeo.bio_geo` (`terrain.rs:376`). Immutable,
  so no write-through concern.
- **`BioCell { last_update, biomass, nutrient }`** (`terrain.rs:357`) → `TerrainState.bio`
  (`terrain.rs:400`), the in-RAM SOURCE OF TRUTH.

**Disk layout is unchanged** via the serde wire-proxy (`TerrainStateWire`, `terrain.rs:417`,
`#[serde(into/from)]` at `:391`) — old 3-Vec layout on disk, packed cells in RAM, no MAGIC bump. Detail:
`determinism.md` §persist. The checksum folds the bio fields field-by-field in the SAME order as before
(padding never enters the hash). This was the **last byte-identical structural lever** — see
`performance.md`.

## The hot getters (`terrain.rs`)

- `biomass_at(x, y, tick)` (`terrain.rs:1043`) → `current_biomass(y*COLS+x, tick)`.
- `current_biomass(i, tick)` (`terrain.rs:1034`) — recovers biomass LAZILY: `regrow` from the value as
  of `last_update` toward `cap_at(i)` over the elapsed ticks. No per-tick global sweep — an untouched
  column costs nothing.
- `cap_at(i)` (`terrain.rs:1011`) — `carrying_capacity(BiomeKind::from_id(g.biome), g.moist/255)` scaled
  by the nutrient pool (Liebig limit).
- `carrying_capacity(biome, moist)` (`terrain.rs:587`), `regrow(b, cap, elapsed)` (`terrain.rs:610`).
- `oxygen_at(x, y, tick)` (`terrain.rs:1104`) — lazy decay toward 0 from `oxygen_update` (closed-form, no
  sweep). The one in-tick read-after-write in apply (`determinism.md` §parallel).

`sense` (`sim.rs`) calls `biomass_at` 4× per creature (own column + 3 offset gradient samples) → the exp
in `regrow` ×4 + the (now-packed) terrain loads. That shape is the perf hot path — `performance.md`.

## The nutrient/oxygen economy (why it's f32, deliberately)

`oxygen` is stored as **f32, NOT quantised** (`terrain.rs:402`): a gentle per-creature O2 drip must
accumulate exactly — a u8/u16 round-modify-write would round each tiny deposit away (this is an
intentional design choice, not an oversight; do not "optimise" it to int). Nutrient is the inorganic pool
(Liebig limit on capacity), grazing carries it off, death returns it, weathering relaxes it lazily via
`nutrient_update`.

## Worldgen = one-time LOAD cost, not per-tick

`tectonics.rs` / `hydrology.rs` / `erosion.rs` / `lem.rs` build `TerrainGeo` once at load. Their cost
(priority-flood, box-blur, the Voronoi pass) is LOAD-time — never confuse it with the per-tick budget
when profiling (a past review mis-flagged worldgen items as tick costs). The world is 1920² columns;
`COLS` indexes them row-major (`y*COLS+x`). `glam` vector ops carry no implicit fast-math
(`external-references.md` §4), so geo math is reproducible across builds.
