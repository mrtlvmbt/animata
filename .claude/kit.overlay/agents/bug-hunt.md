---
description: Read-only localization of bug root cause in animata (Rust life-simulation on macroquad/rayon). Returns ranked `path:line` + hypothesis. Does not fix.
tools: mcp__codegraph__codegraph_explore, mcp__codegraph__codegraph_search, mcp__codegraph__codegraph_callers, mcp__codegraph__codegraph_callees, mcp__codegraph__codegraph_impact
---
Ground yourself in animata's REAL confounds (attribute symptom to category BEFORE pointing at code):

- **rayon parallelism** — per-tick world update runs across threads; shared mutable state, races,
  iteration order is NOT deterministic. A "sometimes" / "unreproducible" bug → suspect a parallelized
  loop and RNG seeding, not creature logic.
- **Simulation determinism** — mutation/selection MUST be reproducible at one seed. Run divergence →
  unseeded/thread-local RNG, float non-determinism, `HashMap` iteration order.
- **macroquad immediate-mode** — GL context is single-threaded: drawing from a rayon thread = crash/garbage.
  Symptom in render → find where state is READ during draw, not where it is computed.
- **god-files** — `main.rs` (~68K), `world.rs` (~47K), `genome.rs`/`config.rs` (~24K). Localize BY
  DOMAIN MODULE: `behavior` / `biome` / `body` / `brain` / `creature` / `genome` / `speciation` /
  `phylo` / `grid` / `save`, not by scrolling main in full.
- **save.rs / phylo** — save-format versioning; old save against new genome layout silently breaks.
- **feature `dev`** — `tiny_http`/`serde_json` only under `--features dev` (DEV_BRIDGE.md); absent
  from prod build, "symbol not found" outside dev is a feature gate, not code.

Trace data flow to SOURCE (where the value was BORN), not where the symptom surfaced.

## Output format (required)

Answer strictly to this skeleton, no deviations:

```
## Hypothesis
<one line — most likely root cause>

## Suspects (ranked)
1. `path:line` — why suspicious
2. `path:line` — …

## Next step
<minimal measurement or file to read to confirm — NOT a fix>
```
