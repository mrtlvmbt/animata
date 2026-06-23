# animata-sim reference library — index & contract

The durable knowledge base for the sim, pulled on demand (progressive disclosure). The spine is
`../SKILL.md`; these files are the depth behind it. **When you spawn a fork-agent
(`bug-hunt` / `subsystem-reviewer` / `critic`) on sim work, pass it the relevant file below in the
prompt** — a cold agent does not see this skill otherwise.

## The volatility split (the rule that keeps this KB from drifting)

Three stores exist; each owns a different *kind* of fact, and **a fact lives in exactly one of them.**

- **This KB (`reference/`) = DURABLE knowledge** — invariants, contracts, architecture, methodology,
  external library facts. Changes only when the **code or a library** changes, so it is versioned in the
  repo *next to the code it describes* (a code change and its doc move in the same PR).
- **Memory (`~/.claude/.../memory/`) = DATED empirical findings** — per-phase ms, the live WIN /
  dead-lever ledger, "current numbers." Inherently time-stamped and volatile.
- **`SKILL.md` = the lean operating spine** — the rules you must follow, with pointers here for depth.

**Consequences (do not violate):**
1. **This KB never restates a volatile number or the live PR ledger.** Where a number is needed, it
   *points* to memory + says "re-measure" — there is no copy to go stale.
2. **Flow is one-directional:** memory (raw, dated) → distil the *durable* lesson into the KB. Never copy
   a KB fact back into memory.
3. **Every internal durable claim cites `(symbol, path)`** (e.g. `cap_at` `terrain.rs`) so a reader — or
   a future grep after a rename — can re-verify it against the code instead of trusting prose that may
   have silently gone stale. **Every external claim** is anchored to a trusted SKILL invariant or a real
   `Cargo.lock` version (see `external-references.md`).

The KB is **append-extensible**: new knowledge = a new file + one row in the table below. No redesign.

## Files

| File | Read it when… |
|------|----------------|
| `architecture.md` | You need the crate boundary, the module map, or the `step` phase pipeline. |
| `determinism.md` | You touch anything that could move the golden — checksum, RNG, parallelism, persist. |
| `terrain.md` | You touch `TerrainGeo`/`TerrainState`, the biomass/oxygen/nutrient getters, or worldgen. |
| `performance.md` | You are optimising the tick at scale — the bench harness, lever taxonomy, the ceiling. |
| `measurement.md` | You are about to benchmark — the iron-rules that stop a fake A/B win. |
| `corridors-and-fragility.md` | A corridor test broke, or you fear a trajectory change will break one. |
| `external-references.md` | You need the *external* fact behind an invariant (FMA, rayon, bincode, glam…). |

## Where do I find X?

- **FMA / debug≠release** → `determinism.md` §profiles + `external-references.md` §1.
- **Why parallel reduce changes bits** → `determinism.md` §parallel + `external-references.md` §2.
- **Save format / MAGIC / wire-proxy** → `determinism.md` §persist + `terrain.md` + `external-references.md` §3.
- **Current per-phase ms / which PR won** → NOT here — memory `sim-perf-100k-scaleup` (re-measure).
- **Why a corridor flips on a seed** → `corridors-and-fragility.md`.
- **How to benchmark without lying to yourself** → `measurement.md`.
