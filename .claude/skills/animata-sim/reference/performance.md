# Performance — the playbook (method & structure, NOT the live numbers)

**Read this when** you are optimising the tick at scale. For the **current per-phase ms and the live
WIN / dead-lever ledger → memory `sim-perf-100k-scaleup`** (re-measure with the bench harness). This file
holds the durable *method*: how to measure, the lever taxonomy, and the ceiling reasoning — none of which
moves when a number does.

## The bench harness

`headless --bench-pop N --profile` (`bench_populate` `sim.rs:640`) seeds N founders via the
profiling-only path (shares the founder loop with `with_config` through `spawn_founders` `sim.rs:579`).
It is **disjoint from the golden** — never in a checksum/acceptance test. `--profile` reports per-phase
ms via the `Span::*` profiler (`sim.rs`). Isolate the run (see `measurement.md`).

## The phase ranking FLIPS with scale

At a few thousand creatures the tick is ~all `decide`. At 100k+ the **serial phases become the wall** and
`apply ≈ decide` (both parallel). Why: genome (`grn_w`, brain) and `Phenotype` are FIXED size regardless
of cell count, so a 32-cell multicellular body costs the SAME per tick as a 1-cell body in decide/apply —
multicellularity's only per-tick delta is `develop()`, which is per-BIRTH, ~0. So "lots of big creatures"
does not change the per-tick shape; population count does.

## The hot-path shape (durable; the ms are in memory)

- **decide** = `sense` (`sim.rs`) + brain `think`. `sense` does 4× `biomass_at`→`current_biomass`→
  `regrow` (an `exp` + terrain loads) per creature; `think` runs the NN with `fastmath::tanh`
  (`fastmath.rs:25`, 10 calls/creature/tick at `sim.rs:176`/`:185`). The threat ring-scan is ~0 for
  non-predators (PR #82 pred-skip gate).
- **apply** = `eval_all` (the float arithmetic of the pressures) + a residual of movement + Kleiber
  `kleiber075` (`fastmath.rs:67`) + 3-4 per-creature RNG rolls + the serial oxygen replay.

## Lever taxonomy (decide which kind you're pulling BEFORE you build it)

1. **Byte-identical structural** — relayout/alloc/struct-size changes that don't move the golden (boxing a
   rare field, reusing scratch buffers, cache-packing co-read terrain fields). The productive vein at
   scale was here: **hot-path struct size / memory traffic in the collect-heavy parallel phases**, not
   dispatch/CSR/genome-layout. The terrain pack (`terrain.md`) was the LAST such lever.
2. **Trajectory-changing** — approximating a transcendental (`fastmath::tanh`/`exp`), cheaper RNG
   derivation, fewer sense samples. Moves the golden → re-pin + the §5 corridor work
   (`corridors-and-fragility.md`). Treat the re-pin as routine cost, not a deterrent (memory
   `golden-repin-is-fine-for-intended-change`).
3. **LOD / update-budget** — act every other tick. The only lever that gives a true *multiple*, but it
   changes the dynamics (a dynamics change, not a free win).

## Dead levers (measured, don't re-litigate without new evidence)

Recorded with their negatives in memory `sim-perf-100k-scaleup`: static enum dispatch (wash — only ~6
well-predicted pressures), CSR grid (pred-skip already neutralised the scan), flat-brain SoA (NEGATIVE —
the bench can't fragment a one-loop-spawned population; brain is FLOPS-bound, not locality-bound), Kleiber
LUT-as-perf (wash; powf wasn't the cost). A full SoA/SIMD campaign is HIGH-RISK with unproven payoff
(touches Creature/genome/checksum/persist/every phase) — don't bet the rewrite blind.

## The ceiling

The work is O(N)-heavy per creature and already parallel ⇒ there is **no structural 3× hiding**.
Constant-factor rewrites of the transcendentals/scatter cap the trajectory package at **~1.2–1.3×**; a
true multiple needs LOD (dynamics change). Measure-first: spike the ceiling on a throwaway branch before
any campaign, and trust only an interleaved A/B (`measurement.md`).
