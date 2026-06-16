# animata — DNA evolution sim

Small artificial-life simulation in Rust + macroquad. Each creature carries an
**ACGT genome** that decodes into body traits (size, speed, sense range,
metabolism, color) **and the weights of a small recurrent neural-net brain**
(11 → 7 → 2, tanh; the hidden layer feeds back into itself, giving a one-step
memory). Creatures sense the nearest food, the nearest threat and the nearest
same-species neighbor, decide how to move, spend energy, split in two when
well-fed, and die when starved. Mutation + natural selection do the rest —
no fitness function, evolution is emergent.

**Diet is a gene** (`carnivory`, 0..1), not a fixed species. Herbivores eat
pellets (energy ×`(1-carnivory)`); carnivores hunt creatures lower on the food
chain (catchable if their carnivory is at least `PREY_MARGIN` below the hunter's),
gaining energy ×`carnivory`; omnivores do both. Carnivores burn energy faster,
sense prey instead of pellets, and are tinted red and slightly larger. The
founding population starts herbivorous (carnivory gene zeroed) to avoid a startup
glut of predators collapsing the world; carnivory then **evolves upward by
mutation** and typically settles into a small, persistent predator fraction
alongside the herbivores — a food chain that emerged rather than being hard-coded.

Creatures also **age**: each carries a longevity gene (a "prime" age). Past their
prime they grow senescent — speed and sense decline and a small, rising chance of
death sets in — so no individual lives forever and generations turn over. Longevity
is an evolving trade-off: longer-lived genomes mature (start reproducing) later, so
under heavy predation fast-breeding short-lifers win, while safe niches reward
longevity (classic r/K life-history selection).

The world is split into **biomes** by a seeded value-noise fertility field —
desert, plains, forest, swamp — each a bundle of food density, movement cost and
metabolic upkeep (a desert is barren + hot; a swamp is lush but sluggish). Food
concentrates in fertile zones, so different regions reward different traits and
locally adapted populations can diverge across the map.

## Run

```bash
cargo run --release        # open the window
cargo test                 # unit + smoke tests
cargo run --example headless          # headless trend print (neural)
cargo run --example headless -- rule  # rule-based behavior
cargo run --example headless -- neural 7  # neural, RNG seed 7
cargo run --release --example sweep 20000 5   # parameter sweep -> sweep.csv
```

Convenience aliases (`.cargo/config.toml`): `cargo play` (release GUI),
`cargo prof neural 1 40000` (per-phase profiling), `cargo sweep 20000 5` (sweep).

## Controls

| Key | Action |
|-----|--------|
| `Space` | pause / resume |
| `Up` / `Down` (or `+` / `-`) | simulation speed (steps per frame) |
| `R` | reset with a new random seed |
| `B` | swap behavior strategy (neural-net ⇄ rule-based) |
| `T` | toggle the live tuning panel (sliders) |
| `S` / `L` | save / load the world to `animata_save.txt` |
| `O` | toggle CSV stats logging to `animata_stats.csv` |
| `G` | cycle creature coloring: diet → lineage → species |
| `Y` | export the ancestry tree to `animata_tree.csv` |
| `P` | toggle the in-app phylogeny tree overlay |
| `D` | toggle the per-diet trait breakdown panel |
| `M` | toggle the Muller plot (lineage shares over time) |
| `C` | recenter / reset the camera |
| Mouse wheel | zoom to cursor |
| Middle-drag | pan |
| Left click | inspect the creature under the cursor |
| Right click | drop food at the cursor |

The bottom panel graphs population and average gene values over time — watch the
traits drift as selection acts. It also plots a **diversity** line (`div`): the
mean per-trait standard deviation across herbivores, so you can see variation
collapse as the population converges on a winning genome, or stay high when
biomes/niches keep distinct sub-populations alive.

`S` writes the whole world (every creature's genome + state, all food, the tick,
the behavior mode) to `animata_save.txt` in the working directory; `L` reloads it,
so you can resume an evolved population across sessions. Phenotypes and brains
are re-derived from the saved genomes.

**Inspecting a creature.** Left-click any creature to open a panel showing its
id, lineage (parent + generation), age/energy, longevity (prime age, maturity,
current senescence %), decoded traits, gene color, a node-link diagram of its
brain (teal/orange edges = positive/negative weights), and its raw ACGT genome. A yellow ring marks the selected individual; the panel
clears when it dies.

**Live tuning.** Press `T` for sliders over `food/step`, `predator gain` and
`mutation rate`, so you can shift the balance while it runs without recompiling.
These map to the runtime `Params` (in `config.rs`) that override the constants.

**Phylogeny / lineages.** Every founder seeds its own lineage id, inherited by
all descendants. The `clade` graph line tracks how many founder lineages still
have living members — it collapses from the starting count toward 1 as lineages
die out and the descendants of a few (eventually one) common ancestor take over
(coalescence). Press `G` to color creatures by lineage instead of gene/diet, so
you can watch clades compete and spread spatially; the inspector shows a
creature's lineage id and its **ancestor chain** (nearest parents first) plus its
full depth, walked from a complete birth/death log (`phylo.rs`) that records every
creature — living and dead. Press `Y` to export that whole tree as an edge list
(`animata_tree.csv`: id, parent, birth, death, lineage) to render in an external
phylogenetics tool. The log is kept bounded by periodic pruning (it only retains
ancestors of living creatures) and isn't saved with the world — on load the
current population becomes the founders of a fresh tree, which then grows again
as evolution continues. (So right after a load the tree is shallow, and its root
count reflects post-load founders rather than the gene-`lineage` count, which is
preserved.)

Press `P` for an **in-app tree view**: it builds the coalescent tree of the
living population (the deduplicated union of everyone's ancestor paths) and draws
it with time (birth tick) left→right, leaves spread vertically, and edges colored
by lineage. As lineages die out you can see the branches converge back toward a
few — eventually one — surviving founder. The living set is sampled so large
populations stay legible.

**Species detection.** Creatures are clustered in normalized phenotype space
(speed, sense, size, metabolism, carnivory, longevity, color) by a threshold
"leader" clustering refreshed every 50 steps (`speciation.rs`): each joins the
nearest species within a distance threshold or founds a new one; centroids that
drift together merge. The detected species count shows in the HUD and the `spec`
graph line, and `G` can color creatures by species. This is distinct from
lineage: one gene-lineage can split into many phenotype species as traits diverge
(e.g. diet), so you'll often see `clades 1` but `species` in the dozens.

Press `M` for a **Muller plot**: each lineage's share of the population stacked
over time (colored by lineage, gray = untracked tail). Bands widen and vanish as
clades rise and die, collapsing toward a single dominant band at coalescence.
Press `D` for a **per-diet breakdown** panel — live average traits split into
herbivore / omnivore / carnivore, so you can see e.g. whether carnivores evolved
faster or larger than herbivores.

**CSV logging.** Press `O` to start appending one row per recorded snapshot
(every 5 ticks) to `animata_stats.csv` — tick, population, herbivore/carnivore
counts, average traits, average carnivory, diversity, max generation. Open it in
any plotting tool to chart a long run (including per-diet population split).
Press `O` again to stop.

**World & camera.** The world (`WORLD_W`×`WORLD_H`) is larger than the on-screen
viewport (`VIEW_W`×`VIEW_H`, the window) — by default 4× the area — so the camera
starts zoomed out to show the whole world and you zoom/pan into it. Food counts
scale with world area (per-biome density unchanged); the starting *creature*
count does not, so a fresh world is sparse and grows to fill the space. Sim cost
scales with population, so very high speed multipliers in a fully-populated large
world will dip below 60 fps — lower the speed or shrink `WORLD_*`.

**Camera.** Mouse wheel zooms toward the cursor, middle-drag pans, `C` recenters
— handy for watching one individual or a cluster of related (similarly colored)
creatures up close. Lineage shows up directly in the colors: the color is a
heritable gene, so kin share a hue and it drifts as mutations accumulate.

## Layout

- `config.rs` — all tunable constants (world, energy, mutation, NN topology).
- `genome.rs` — ACGT genome: random / mutate / decode to phenotype.
- `behavior.rs` — pluggable decision strategy: the `Behavior` trait, the
  `BehaviorKind` selector, and the neural-net / rule-based implementations.
- `brain.rs` — fixed-topology recurrent network (Elman-style) used by the neural behavior.
- `creature.rs` — one organism + its `Species` (herbivore / predator): sense →
  decide → act → metabolism → reproduce.
- `grid.rs` — uniform spatial grid for O(1)-ish nearest-neighbour lookups
  (food, prey, predators), so the sim scales to thousands of creatures.
- `biome.rs` — seeded value-noise fertility field + biome classification and
  per-biome properties (food/move/metabolism multipliers and tint).
- `save.rs` — text save/load of the whole world (genome + state), no extra deps.
- `phylo.rs` — ancestry log (births/deaths of all creatures) for the family tree.
- `speciation.rs` — phenotype-space clustering into detected species.
- `world.rs` — sim step: food spawn, eating, reproduction, death, stats.
- `stats.rs` — rolling history for the trend graph.
- `main.rs` — window, rendering, input.

## Genome & reproduction

The genome is a string of ACGT nucleotides decoded in groups of 4 (a base-4
number, `0..=255`) into body genes then neural-net weights. Three mutation types
act on it at reproduction:

- **substitution** — a nucleotide flips (common, small effect);
- **insertion / deletion (indel)** — the genome grows or shrinks, shifting every
  downstream gene (frameshift, rare and disruptive); length is clamped to a band.

Decoding tolerates any length (missing nucleotides read as 0), so a phenotype is
always well-formed. Reproduction is **sexual** when a fertile same-species
partner is within range — single-point crossover of the two genomes, then
mutation — and falls back to **asexual cloning** otherwise.

## Communication

The brain has an extra **output** — a `signal` (call loudness, 0..1) — and an
extra **input** — the loudness it `hears` from its nearest neighbor (last step).
Nothing is hard-coded: with these wired into the evolving net, behaviours like
alarm calls (emit when a threat is near) and reactions (flee when you hear a call)
can emerge through selection. Loud signallers get a faint white ring; the `sig`
graph line tracks mean signal and the inspector shows a creature's current call.
(The rule-based brain has a hard-wired alarm: it calls when threatened and flees
when it hears one.)

## Sexual selection

Two more genes drive mate choice: an **ornament** (display trait) and a
**preference** for it. When picking a mate, a creature scores nearby fertile
candidates by `its preference × their ornament`, so showy mates are chosen more
often — a Fisherian runaway. The ornament is a **handicap**: it raises metabolic
upkeep (`ORNAMENT_COST`), so it would decay to zero without sexual selection.
With it, the ornament stays elevated and fluctuates in a tug-of-war between mate
choice and survival cost. Watch the `ornm` graph line and the inspector's
`ornament`/`pref` values.

## Behaviors

Decision-making is a swappable strategy (the `Behavior` trait in `behavior.rs`):

- **neural-net** — recurrent net (11 → 7 → 2; hidden state feeds back for a
  one-step memory), weights decoded from DNA.
- **rule-based** — steer toward the nearest visible food, wander otherwise; the
  steering gains are also decoded from DNA, so it still evolves.

Both read genes, so selection acts on either — but the *pressure differs*: the
neural net rewards larger sense range, while the rule-based version trims it
(it steers perfectly regardless of range, so vision is mostly extra cost).

Add a variant: implement `Behavior`, add a `BehaviorKind` arm, wire it into
`BehaviorKind::build`. Toggle live with `B`, or pass `rule` to the headless
example.

## Biomes

A seeded value-noise field (`biome.rs`) gives every point a fertility in `0..1`,
classified by threshold into **desert / plains / forest / swamp**. Each biome
bundles three effects: food spawn density (food is rejection-sampled toward
fertile zones), a movement-distance multiplier (swamp is sluggish), and a
metabolic-upkeep multiplier (desert is hot). The map is drawn as a translucent
background tint and is derived from a seed stored with the world (so it reloads
identically). Tune the bundles and thresholds in `biome.rs` / `config.rs`.

Because food clusters by biome and offspring stay near their parents, regions
reward different traits — watch the population diverge locally over time.

## Tuning

Edit `config.rs`. Lower `FOOD_PER_STEP` to make the world harsher (selection
sharpens); raise `MUTATION_RATE` for faster, noisier drift, or `INDEL_RATE` for
more frameshift upheaval (predators are sensitive to it). Change
`DEFAULT_BEHAVIOR` in `behavior.rs` to pick the startup strategy.

Predator balance is sensitive (it's a chaotic predator–prey system):
`PREDATOR_GAIN` (energy per kill), `PREDATOR_METAB_MULT` (how fast they starve),
and `PREDATOR_SPEED_MULT` (raising it tips the world toward predators and tends
to wipe out prey).

**Aging vs predators.** With aging on, herbivores stay robust (no extinctions
across tested seeds) but predators consistently die out: aging adds mortality and
a maturity delay that their small, marginal numbers can't absorb. This is an
inherent tension, not a tuning miss — once aging weakens the prey base (slower,
dimmer-sensed old prey), *boosting* predators to compensate (`PREDATOR_LONGEVITY_MULT`,
faster predator maturity) makes long-lived predators over-hunt the weakened prey
into a mutual collapse. So the default keeps predator aging neutral and accepts
that predators fade once aging is in play; raise `PREDATOR_LONGEVITY_MULT` if you
prefer lively-but-fragile predators (with occasional total ecosystem collapse).

## Environment (seasons & droughts)

Food spawn is modulated by a global environment: a slow **seasonal** sine wave
(±~22% over `SEASON_PERIOD` steps) plus stochastic **droughts** that sharply cut
food spawn for a few hundred steps. Both push population booms and busts and
select for efficiency. The HUD shows the current state (`bounty` / `mild` /
`lean` / `DROUGHT`) and the world gets a parched tint during a drought. Tunables
(`SEASON_*`, `DROUGHT_*`) live in `config.rs`; defaults are calibrated so the
population always survives across seeds while still oscillating.

## Performance

Rendering is batched for scale: the biome map is baked once into a texture (one
quad/frame instead of hundreds of rectangles) and rebuilt only on reset/load; all
food and creatures are emitted into a single `Mesh` and drawn in one `draw_mesh`
call (food as quads, not tessellated circles), with off-screen entities culled.
The HUD shows frame time (ms) and the drawn-entity count next to FPS.

The simulation step is profiled per phase (`World::profile`); the headless example
prints a µs/step breakdown. Sensing was the hot phase — every nearest-neighbour
query is now bounded by the searcher's sense range (`SpatialGrid::nearest_within`)
instead of scanning the whole grid when no match is nearby, which cut the step
time ~4–5× (sensing 407 → 75 µs/step) so high speed multipliers stay smooth.
Smaller follow-ups: the biome is classified into a lookup grid once (no
value-noise per creature/step), and the threat + neighbour searches share one
grid traversal (`nearest2_within`). Per-step scratch (both spatial grids and the
position/diet/target buffers) is pooled and reused, so a step does essentially no
heap allocation. Step time is ~90–100 µs at a few hundred creatures — comfortably
within budget even at speed ×40.

For large populations the two heavy phases run in parallel (rayon): the read-only
**sense** pass and — for the neural brain — the **act** pass, since both are
per-creature independent and RNG-free (the rule-based brain's wander uses the
global RNG, so it stays serial). Each roughly halves, keeping high speed smooth
in a fully-populated large world.

**Always run `--release`.** The debug build is ~13× slower at the per-step math
(neural-net forward + neighbour search), so high speed multipliers will stutter
in a debug build but run smoothly in release. The HUD shows frame time (ms) and
the drawn count; `cargo run --example headless -- neural <seed> <steps>` prints a
per-phase µs/step breakdown for profiling.

## Parameter sweeps

`examples/sweep.rs` (`cargo sweep [steps] [seeds]`) runs the sim headless across a
grid of parameters (`food_per_step` × `mutation_rate`) × seeds, single-threaded
(macroquad's RNG is global), and writes one outcome row per run to `sweep.csv`
(survival, steps survived, final population, traits, diversity, species, clades,
max generation). Use it to map balance — e.g. which food rate causes extinctions,
or how mutation rate trades off diversity vs convergence — instead of guessing.

## Possible next steps

Candidate directions: host–parasite co-evolution, richer terrain (barriers →
allopatric speciation), recurrent-memory analysis, and UX polish (help overlay,
config presets, color legend).
