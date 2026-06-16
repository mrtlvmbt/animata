# Macroevolution overhaul — architecture plan

From **microevolution** (tuning radius/speed sliders) to **macroevolution**
(evolvable body plans — worms, fish, insects, later birds). Grounded in the
a-life literature (Karl Sims / Framsticks evolvable morphology; NEAT-style
marker encoding; Morphological Innovation Protection for the fragile
body↔controller co-evolution problem).

---

## Decisions locked (from discussion)

- **Locomotion = capability abstraction, not joint physics (fork 1).** Body →
  pure function → aggregate numbers (thrust, drag, mass, efficiency) per medium.
  Brain still emits `throttle`/`turn`; those scale by the body's numbers.
  *No mass-spring simulation.* Movement looks like today's gliding (optional
  cosmetic body-wave on top).
- **Seam left for future joint physics (fork 2).** A `Locomotor` boundary:
  `Body::locomotion(medium) -> LocomotionStats`. Capability impl computes
  analytically now; a future physical impl steps joints — swap without touching
  callers. Brain ports work for both (port = "appendage actuator" now, "joint
  torque" later).
- **Vertical = discrete layers, MORE than one, animal owns a *subset* (fork on
  layers).** Stack (extensible), per-animal `layer_access` bitmask derived from
  morphology:

  ```
  5  high sky      strong flight / soaring
  4  low air       flight, nectar above flowers
  3  canopy        climb / light flight, fruit
  2  understory    climbing
  1  surface       BASE — walk / crawl  (always exists)
  0  underground   burrow — roots, benthic food
  ```

  In `Water` biome the same index reads as a water column (surface/mid/bottom).
  Which layers *exist* at a point depends on biome (canopy only in Forest, 0
  everywhere, water layers only in Water). Movement gets a brain actuator
  "change layer ±1", allowed only if the neighbor layer is in the mask; costs
  energy, one step at a time. Sense/eat/hunt/mate happen within the creature's
  current layer (optionally predators may strike ±1 — "stooping hawk").

## Open forks — proceeding on the **recommended default**, flip any if wrong

- **Iteration scope:** Phase 0 + Phase 1 (foundation; body = 1 segment =
  today's circle; behavior byte-identical, 23 tests stay green, 8/8 seeds
  survive). *Not* jumping straight to segmented bodies.
- **Start morphotypes:** land + water (worms / fish / insects) first, flight
  later — but the layer stack above already reserves air layers so adding flight
  is data, not rearchitecture.
- **Map scale:** medium **×16** of today's world (~5–10k creatures, tractable),
  with render LOD so the "giant map, zoom & scroll" view works. *Not* ×100+
  (that needs world chunking — deferred).

---

## The four mechanisms (grounded in current code)

### 1. Graph/chain morphology — replaces scalar `radius`

Today `Phenotype` (`genome.rs:20-41`) is flat; body is a point with `radius`.
New: body is a **chain of segments** decoded from the genome.

```rust
struct Segment {
    length: f32,
    width: f32,
    appendage: Appendage,   // None | Fin | Wing | Leg | Burrow
    flexibility: f32,       // bend vs previous segment
}
enum Appendage { None, Fin, Wing, Leg, Burrow }

struct Body { segments: Vec<Segment> }
```

Morphotype = emergent pattern: long chain + `None` = worm (peristalsis, cheap,
high traversal); chain + `Fin` = fish; short stiff body + `Leg` = insect; `Wing`
= flyer. `Body` derives the old scalars (`radius` ≈ bounding size, `max_speed` ≈
thrust/drag) so the rest of the sim keeps working through a compatibility layer.

### 2. Medium physics in biomes — the selection pressure

Today biomes are passive `move_mult`/`metab_mult` (`biome.rs:20-32`). Add a
**medium** (Air / Ground / Water) per biome+layer. `Body::locomotion(medium)`
returns thrust/drag/mass; the env makes the wrong body expensive:

- **Water:** no `Fin` → huge drag, ~5× move energy → pressure to evolve fins.
- **Air layers:** ground bodies get `move_mult ≈ 0` (impassable); enter only
  with `Wing` generating `Lift > Gravity`.

This is what turns "worm wants to become fish/bird" into a real gradient.

### 3. Co-evolution of brain & body — dynamic I/O ports (THE hard part)

Today `NN_INPUTS=12`, `NN_OUTPUTS=3` are constants (`config.rs:232-235`) and
weights are packed by fixed index. A new appendage has nowhere to plug in, and
an indel frameshifts every weight (`genome.rs:53-73`).

**Fix = marker / tag-based encoding** (NEAT-style; this is the keystone deferred
from the last review — now promoted to Phase 1):

- Genome is parsed by **start codons** delimiting variable-count records:
  segments, neurons, synapses. Each sensor/actuator a segment grows
  **auto-registers a port id**: fin → output `Thrust_Fin`; wing → output
  `Flap_Wing` + input `Airspeed`.
- The brain is **assembled by matching tags**, not array offsets: a synapse
  record names source/target *tags*, wired up at build time. So a body that
  grows a wing finds (or mutates) synapses that drive it; indels add/drop whole
  records instead of shifting all weights → mutation stops being "constant
  lobotomy."
- **Founder compatibility:** a single-segment body with the fixed sensor/actuator
  set decodes to *exactly* today's 12→7→3 brain and today's phenotype. Phase 1
  ships this path only → behavior identical, determinism preserved, tests green.

**Fragility mitigation** (literature's two answers to "body mutates, controller
breaks"), both baked in:
- **Morphological Innovation Protection:** a freshly body-mutated lineage is
  shielded from selection for N steps so control can re-adapt before it's judged.
- **Modular/local control:** per-segment controller sub-blocks survive body
  edits (a kept segment keeps its working control).

### 4. Ecological niche divergence — tiered resources

If everyone eats the same pellets, one metabolic optimum wins and bodies never
split. Bind food to **(biome, layer, flavor)** on top of today's flavor/niche
system (`config.rs:134-152`, already good):

- **Benthic/underground (worms):** food at layer 0; needs `Burrow`.
- **Canopy fruit (birds/insects):** food at layer 3 in Forest; reachable only by
  climb/flight.
- **Nectar (insects):** fast-decaying high-value points; rewards maneuver speed.

---

## Speciation & stats under dynamic bodies

Leader clustering on a flat feature vector (`speciation.rs`) breaks when body
topology varies. Move to **topological distance**: segment-count + appendage
multiset + trait deltas (a cheap tree/graph-edit-distance approximation). Stats
(`stats.rs`) then show real macroevolutionary turnover — whole classes rising
and going extinct (e.g. a drought flips biomes, hydrodynamic clade gives way to
arthropod clade), not just a wobbling mean speed.

---

## Phasing

> **Progress:**
> - **Phase 0 DONE.** LOD dot rendering (`main.rs` `draw_entities`,
>   `LOD_POINT_PX`) + giant map ×16 (`config.rs`: WORLD 8800×6080, food/founders
>   scaled, POP_CAP 12000). The `project()` seam is already satisfied by
>   `world_to_screen` (one fn to swap for isometry); per-cell heatmap LOD judged
>   premature (12k dot-quads batch fine). Headless: equilibrium ~4.5k–12k
>   (grazes POP_CAP at peaks), ~4ms/step peak, no extinction. *GUI visual check of
>   LOD still pending (no display in dev env).* Ornament settled ~0.37; sim
>   integration test now ~60s.
> - **Phase 1 sub-step 1 DONE — `Locomotor` seam (fork-2 hook).** New `body.rs`:
>   `Medium`/`LocomotionStats`/`Locomotor` trait; `creature.rs` movement routes
>   thrust through `pheno.locomotion(Medium::Ground)`. Byte-identical (seed 6
>   reproduces 1536/2933/8212/11835 exactly), 23 tests green, bin clean.
> - **Phase 1 sub-step 2 DONE — marker/tag genome (the keystone).** Branch
>   `phase-1-marker-genome`. Brain weights are no longer a fixed contiguous block:
>   the genome holds start-codon-delimited **synapse records** (`SYNAPSE_START`,
>   src/dst port tags + weight), nt-scanned at any frame so indels add/drop whole
>   synapses instead of frameshifting every weight. `Brain::from_synapses` routes
>   records into dense matrices (forward stays a matmul). Founders constructed via
>   `Genome::random()` (dense 154-conn brain). Encoding change re-rolls the
>   RNG→genome map, so exact trajectory numbers differ (expected). Validated: 8/8
>   seeds survive 4000 steps (pop 1.5k–11.8k, predators evolve, species 83–419,
>   mem ~0.48), 23 tests green, bin clean.
> - **NOT YET (still Phase 1/2 keystone work):** *dynamic* ports from body-grown
>   sensors/actuators (currently fixed 12/7/3 port set); evolvable hidden-neuron
>   count; segment chain in the genome. These land with Phase 2 bodies.
> - **Phase 2.1 DONE — segment-chain genome + render.** Branch
>   `phase-2-segmented-bodies`. Refactored the marker stream to **unified typed
>   records** (`RECORD_START` + type gene: `REC_SYNAPSE`/`REC_SEGMENT`), one scan
>   that skips record interiors — fixes cross-talk (a synapse record's bytes can't
>   spawn a spurious segment). `Segment`/`Appendage` decode into
>   `Phenotype.segments`; body bounding radius derives from the chain
>   (`body_radius`); founders emit zero segments (== old circle). Render draws the
>   segment chain as a quad row with appendage tints (`segment_layout`,
>   `appendage_tint`). Validated: 8/8 seeds survive, 24 tests green, bin clean.
>   *Caveat:* segments are currently selectively neutral/costly, so morphotypes
>   won't emerge until 2.2 gives appendages locomotor payoff. Flaky giant-world
>   sim test seen once (borderline threshold) — watch.
> - **Phase 2.2 DONE — medium physics + appendage locomotion.** Biome → `Medium`
>   (`biome.medium()`); `Locomotor::locomotion(medium)` scales thrust by how the
>   body's appendages suit the medium (legs→ground, fins→water, wings→air) with
>   diminishing returns; per-segment/appendage upkeep (`SEGMENT_UPKEEP`,
>   `APPENDAGE_UPKEEP`) gives an interior optimum. Two balance fixes that mattered:
>   (a) segments are a *rare* type-gene band (`SEGMENT_TYPE_MIN`) so body plans
>   change by rare macro-mutation, not a mutational flood that slams the cap;
>   (b) bounding radius derives from segment *width* only, not chain length, so
>   long bodies don't win free eating reach. Stats `avg_segments`/`appendaged_frac`
>   added (panel + headless). Result: 8/8 seeds survive (pop 5.9k–10.8k), legs
>   evolve from zero to ~40–59% adoption at avg ~1 segment — genuine selection, no
>   runaway, no collapse. 24 tests green; bin clean. Branch
>   `phase-2-segmented-bodies` (2.1 already merged to main).
>   *Note:* fins/wings have capability but little purpose until aquatic/aerial food
>   exists — that's 2.3.
> - **Phase 2.3 DONE — vertical layers + tiered foraging.** 3 layers
>   (underground/surface/air); a creature's layer is its morphological stratum
>   (`Phenotype::primary_layer`: wings→air, burrow→underground, else surface).
>   Surface keeps positioned pellets (baseline untouched); non-surface layers offer
>   a fixed foraging **capacity split among occupants** (`BENTHIC_CAPACITY`,
>   `AERIAL_CAPACITY`) — density-dependent, self-limiting, no food-vector/save
>   churn. Sensing/eating/hunting gated to a creature's layer (so a stratum is a
>   predator refuge) via grid predicates on `clayers`. Stats `frac_underground`/
>   `frac_air` (panel + headless); creatures tinted by layer. Result: three strata
>   coexist (~57% surface / ~27% burrowers / ~16% fliers), population thriving
>   (caps 12k), 400+ species — genuine vertical niche divergence, no collapse.
>   *Simplifications to revisit:* layer is fixed by morphology (no in-life movement
>   between layers yet); mating/infection not layer-gated; non-surface food is
>   abstract yield, not positioned pellets.
> - **NEXT — Phase 2 wrap / Phase 4: topological speciation** (cluster by body
>   plan, not flat traits) + macroevolution stats; then optional richer layer
>   movement / aquatic pellets.

- **Phase 0 — Render decoupling + giant map + LOD (zero sim change).**
  Introduce `project()` seam (top-down now; isometry later = swap one fn),
  `Body::shapes()` draw primitives, and `layer` tint/shadow. Scale world ×16 and
  add density LOD: far/overview = 1px point or per-cell heatmap; near = full
  body. Reuses existing `View`/`world_to_screen`/batched mesh
  (`main.rs:62-98,402`). The "giant map + zoom/scroll" is just a third consumer
  of `project()` + `Body::shapes()`.
- **Phase 1 — Marker genome + `Locomotor` seam + body = 1 segment (KEYSTONE,
  invisible).** Tag-based decode & dynamic brain assembly, founders byte-identical
  to today. Verify: determinism, 8/8 seeds survive 4000 steps, 23 tests green.
- **Phase 2 — Multi-segment bodies + appendages + medium physics + layers.**
  Worm/fish/insect morphotypes emerge; Water medium drag/thrust; burrow→layer 0;
  legs→layer 1–2. (Needs the command-buffer eat/hunt parallelism here for 5–10k
  pop — see deferred.)
- **Phase 3 — Air layers + flight.** Wings/lift, canopy & nectar niches, evolving
  `layer_access`.
- **Phase 4 — Topological speciation + macroevolution stats.** Class
  emergence/extinction over time; coalescent/monophyletic clades.

---

## Critical items still unimplemented (carried forward, per request)

- **Tag/marker synapse encoding** — *promoted from "deferred (big)" to Phase 1
  keystone.* Everything above depends on it.
- **Command-buffer `eat`/`hunt`** — was "too early"; becomes required at Phase 2
  once population reaches 5–10k (parallel resolution without races).
- **Full coalescent / monophyletic speciation** — was "expensive"; folds into
  Phase 4 topological speciation.
- **World chunking** — only needed if map goes ×100+ (currently deferred; ×16
  does not need it).

---

## Architectural seams that keep future change cheap

- `Locomotor` trait → capability now, joint physics later (fork-2 ready).
- `project()` → top-down now, isometric/3D later (one function).
- Tag-based ports → port = appendage actuator now, joint torque later.
- `layer: u8` + `layer_access: u16` mask → world stays 2D XY; verticality is
  data, render stacks later.
