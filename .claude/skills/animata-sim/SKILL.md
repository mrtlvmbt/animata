---
name: animata-sim
description: The operating manual for the animata evolution simulation (crates/animata-sim). Invoke at the START of ANY work touching the sim — development/GRN, genome, selection pressures, metrics, save/load, determinism, the golden checksum, acceptance corridors, or the morphogenesis program. Encodes the determinism contract, the exact re-pin / output-capture / review procedures, and the hard-won fragility lessons so you don't rediscover them. Use it before editing sim code, not after a test breaks.
---

# animata-sim — the simulation operating manual

A deterministic Rust evolution sim. **Determinism is sacred; the golden checksum is the lock.** Most
mistakes here are determinism leaks or trajectory perturbations that silently break a tuned corridor.
This skill is the spine: read it, then act. It pairs with the memories (`MEMORY.md`): especially
[[morphogenesis-program]], [[working-style]], [[refactor-program]], [[adding-a-pressure]],
[[cold-agents-need-verification]], [[review-before-merge]], [[always-pr-to-main]],
[[save-load-render-lod]], [[dev-bridge-port]], [[verify-visual-fixes-in-app]].

## 0. The one rule that dominates everything

**The sim must replay BIT-IDENTICALLY within a profile.** `state_checksum(&Sim, &VoxelTerrain) -> u64`
(`sim.rs`) folds the FULL state (every creature field + every genome `Vec<f32>` + all mutable terrain)
and a 300-tick run on seed 42 must equal `GOLDEN_CHECKSUM_SEED42_300`. If you change behaviour, the
golden changes — that is expected, you re-pin it (§3). If you DIDN'T mean to change behaviour and the
golden moved, you have a bug — find it, don't re-pin.

Corollaries you must obey:
- **Never float-add into a hash or a determinism-critical aggregate.** Use `f32::to_bits` + integer
  fold (FNV/splitmix). Float `+` is not associative ⇒ rayon/reorder gives different bits.
- **debug ≠ release.** LLVM fuses `a*b+c` into FMA in release, not debug, so the two profiles diverge
  over thousands of ticks. The golden is pinned PER PROFILE via `cfg!(debug_assertions)`. **Canonical
  verification profile = release** (acceptance corridors are tuned there). A "green PR" = a green
  `--release` run; debug golden must also hold but corridors aren't tuned for debug.
- **RNG only via `seed_fold(world_seed, &[SALT, id, tick])`** (or splitmix on `(id,tick)`) — never
  wall-clock, never iteration-order-dependent. A per-pair roll (predator i × prey j × tick) stays a
  per-pair roll; don't collapse it.
- **Parallel only the read-only decide phase** (`into_par_iter` writes `decisions[i]` per index, then
  collects in index order). Mutation (births, deaths, deposit, terrain) is SERIAL in fixed index
  order. Any float aggregate on the critical path is integer or a serial fixed-order sum.

## 1. Architecture (where things live)

- **`crates/animata-sim`** — the lib: `sim.rs` (step, Creature, state_checksum, golden, metrics),
  `genome.rs` (GRN development), `terrain.rs` (Arc<TerrainGeo> immutable + TerrainState mutable),
  `grid.rs` (spatial), `rng.rs`, `config.rs` (tunable consts), `persist.rs` (save/load), `clock.rs`,
  pressure/metric registries. **No macroquad** (uses `glam`). `bin/headless.rs` runs it without graphics.
- **`crates/animata`** — the bin: render (`main.rs` + `render/`), `dev_bridge.rs` (behind `--features
  dev`), UI. Reads the sim through a thin seam; never owns sim logic.
- **The boundary is enforced by the compiler** (separate crates). Keep sim logic in `animata-sim`;
  keep rendering/IO in `animata`.

## 2. The development model (GRN → body) — read before touching `genome.rs`

A genome is a **gene-regulatory network**, not a body blueprint. Development grows a body from one
seed cell over `DEV_STEPS` steps: each cell runs `s' = tanh(W·s + b)` (`regulate`), divides when
`GENE_DIVIDE > DIVIDE_THETA` (daughter gets a polarity flip), capped at `MAX_CELLS=32`. Cell type =
argmax of the 7 function genes (`cell_type`). **C0 continuity by construction:** the founder's GRN is
all-zero ⇒ `tanh(0)=0` ⇒ one structural cell ⇒ the C0 organism. Mutation grows the GRN away from there.

Pillars (do not violate — they protect perf + determinism):
- **Develop-time, frozen to INTEGER stats.** `develop() -> Phenotype` runs ~4×/tick (per birth); the
  hot path (2.3 ms/tick @ 6000 creatures) reads only integer `Phenotype` fields. **Never store
  coordinates on `Phenotype`** (kills `Copy`, 12k heap allocs) — re-derive them for render.
- **One shared core `grow()`** feeds both `develop()` (counts) and `body_layout()` (render) — so the
  drawn body always matches the stats. Render reaches it via `Creature::body_layout_for_render()`
  (genome stays private). Render RE-GROWS every frame per visible creature — if `grow()` ever gets
  expensive, cache the layout IN THE RENDER LAYER keyed by a genome hash, never on `Creature` (serde!).
- **`organ_power(type) = count + ORGAN_BONUS·max(0, largest_cluster−1)`** — monotone, no fitness
  valley. Organ-driven stats so far: effector→speed, storage→max_energy, sensor→sense reach. Add new
  organ effects through `organ_power`, not raw counts, to reward coherent tissue.
- **Morphogenesis Phase 2 (morphogen gradients)** is mid-build — see [[morphogenesis-program]] for the
  exact state (PR-D1 landed the diffusion machinery INERT; PR-D2 switches the coupling on). The plan is
  `~/.claude/plans/morphogen-gradients.md`.

## 3. Re-pinning the golden (the exact procedure)

When a behavioural change shifts the trajectory, BOTH profile goldens move. Read the new values:

```sh
# RELEASE golden — from CI (the gate runs release on the matched arch). Push, then the golden-arm64
# job's failure carries the assert left:/right:.
git push && bash scripts/ci-report.sh        # on failure: read .ci-report/failed.log
grep -iE "left:|right:" .ci-report/failed.log
# DEBUG golden — CI runs release only, so read the debug value from a TARGETED local run on the dev
# machine (the allowed local use, §4); on the arm64 dev box it matches the pinned debug golden.
./scripts/test-bar.sh -p animata-sim state_checksum_replays_to_golden | grep -iE "left:|right:"
```

`left:` is the actual (new) value, `right:` is the stale golden. Paste both into
`GOLDEN_CHECKSUM_SEED42_300` in `sim.rs` (debug branch + release branch), with a comment saying WHY it
moved (same for `GOLDEN_CHECKSUM_SEED1_8000`). Then push and confirm `ci-report.sh` exits 0.

**Before re-pinning, ask: did I MEAN to move the trajectory?** If the change was supposed to be inert
(machinery only), a moved golden means it leaked — fix the leak (§5), don't re-pin. The legitimate
inert re-pin is *only* when you ADDED a field to the checksum fold (the hash inputs grew, the
trajectory didn't).

## 4. Running tests — the cloud CI pipeline is the gate

**The authoritative green gate is CI, not a local run (CLAUDE.md).** Standard loop: commit → `git
push` → `bash scripts/ci-report.sh`; **merge ONLY on exit 0**. On failure read `.ci-report/failed.log`
(panic body, assert `left:`/`right:`) + `.ci-report/artifacts/*/junit.xml` (which tests). CI is two
jobs, per-arch — see [[ci-push-triggered]]; it covers **`animata-sim`** (the corridors + the 3
golden locks), not the render bin. **Do NOT run the full `./scripts/test-bar.sh` suite locally** — that
is exactly the machine load CI removes.

**Local `./scripts/test-bar.sh` stays available but OPTIONAL — only for fast targeted iteration** on
one test while developing (`./scripts/test-bar.sh -p animata-sim --release state_checksum`); never bare
`cargo test`. It wraps cargo (runs raw cargo internally so the rtk proxy doesn't swallow output),
honours `.cargo/config.toml`'s `RUST_TEST_THREADS=1`, passes failure detail through; non-TTY → periodic
checkpoint lines instead of a `\r` bar (cadence `BAR_EVERY=N`).
- The 8000-tick corridors run ~14 s each in release (~6 min full local / ~18 min on the x86 CI runner).
  Let CI carry the full suite; keep any local run to the single test you're iterating on.
- Fallback if the script is unavailable: `rtk proxy cargo test ... -- --nocapture`, or the tee log
  `~/Library/Application Support/rtk/tee/<ts>_cargo_test.log`.

## 5. The fragility lesson (single-seed corridors) — the #1 way to break things

Acceptance corridors (`camouflage_emerges`, `toxin_resistance_evolves`, `organs_emerge`,
`predation_emerges`, `multicellularity_emerges`, `seasonality_…`, speciation) each run ONE seed for
8000 ticks and assert an emergent statistic clears a threshold. **They are brittle to ANY trajectory
perturbation** — even a change that adds genes shifts the mutation RNG stream and reshuffles which
single-seed corridor passes. Observed: a genome-widening change knocked `toxin` seed-1 to 0.009 (mean
over seeds 1–5 was a healthy 0.113) while `crypsis` rose to 0.188. This is seed luck, not mechanism.

Defences, in order of preference:
1. **Make the change inert** (§ below) so the trajectory is byte-identical — corridors untouched.
2. **Preserve the RNG stream of existing genes**: when adding genome fields, draw the new fields LAST
   in `mutate` (struct-literal fields evaluate top-to-bottom) and as constants in `founder` (no rng),
   so existing genes keep their exact draw sequence. This alone recovered crypsis from 0.032→0.188.
3. **If a corridor still wobbles, prove the mechanism survives across seeds** (probe seeds 1–5 with a
   temporary `#[ignore]` test) and make the corridor MULTI-SEED robust (assert the mean, or "≥k of n
   seeds clear the bar"). This is a legitimate robustness fix, documented as such — NOT cherry-picking
   a passing seed. Never just lower a single-seed threshold to make your PR pass.

## 6. The "land it inert, switch it on later" pattern (proven: PR-A/B/C, PR-D1)

For risky determinism-critical additions, split into (a) **inert machinery** PR and (b) **activation**
PR. Inert = the new code path produces a byte-identical trajectory:
- New genome fields: `founder` sets them as constants (no rng); `mutate` does NOT draw them (clone
  through) and the field that gates the new behaviour starts at a no-op value (e.g. `morph_w=0` ⇒ the
  morphogen is never read ⇒ `regulate` output unchanged).
- Guard expensive new compute behind "is this feature actually engaged?" (e.g. skip diffusion when
  `morph_w` is all-zero) so inert bodies pay nothing.
- The golden moves ONLY because the checksum now folds the new (constant) fields — re-pin, all
  corridors green, no test changes. The activation PR then mutates the gates, re-pins, and does the
  corridor-robustness work as its explicit job.

## 7. Checklist when adding a new Phenotype / Genome field (F9 — easy to forget)

`state_checksum` folds `Phenotype` by a HAND-WRITTEN field list (`sim.rs`), and `Genome::checksum`
folds genome `Vec`s by hand — neither is derived. So a new field silently escapes the determinism lock
unless you:
- [ ] add the new `Phenotype` field to the manual fold in `sim.rs` (next to `p.organ`).
- [ ] add new `Genome` `Vec`/scalar to `Genome::checksum` in `genome.rs`.
- [ ] **bump the save MAGIC** in `persist.rs` (`bincode` is positional — a new field anywhere shifts
      every following byte, so pre-change snapshots must be cleanly rejected, not mis-decoded).
- [ ] re-pin BOTH the determinism golden AND verify `snapshot_round_trips_bit_identical` (save/load).

## 8. Adding a selection pressure

See [[adding-a-pressure]] (toxicity #35 is the worked example). A pressure is a pure
`eval(env, pheno, genome, ctx) -> Effect`; the Effect writes only into the fixed channel dictionary
(`food_mult`, `energy_add`, `metab_mult`, `detection_bias`, `mortality_add`, `repro_mult`); composition
is commutative ⇒ order-independent ⇒ determinism safe. Density-dependent aggregates go in a finished
pre-pass (`TickCtx`, integer or serial sum). Environment feedback (deposit) resolves in the serial
apply phase — never give a pressure `&mut terrain` in eval. New pressure = new module + registration;
`step` is not surgery. Add an acceptance test (mind §5 fragility).

## 9. Workflow gates (non-negotiable)

- **Big feature / architecture / new mechanism → plan-consensus FIRST** (`/plan-consensus`, the critic
  loop) before writing code. Land the hardened plan in `~/.claude/plans/`.
- **Determinism-critical or behavioural change → subsystem-reviewer BEFORE merge** (mandatory per
  [[review-before-merge]]). Fix every FAIL, re-confirm, then merge. Docs/test-only changes don't need
  it (state why).
- **CI green is the merge gate, not a local run** — push, `bash scripts/ci-report.sh`, merge ONLY on
  exit 0 (§4, [[ci-push-triggered]]). Do NOT run the full local suite as the gate (CLAUDE.md).
- **Land on main ONLY via a GitHub PR** ([[always-pr-to-main]]). Create the branch in a SEPARATE Bash
  call first (a guard hook blocks committing on main even inside a `checkout -b && commit` compound),
  confirm `git rev-parse --abbrev-ref HEAD`, then commit. Don't stage `.claude-dev-kit` (submodule).
  Delete the branch after merge; sync local main.
- **Determinism-critical files (`sim.step`, `genome`, terrain mutation) are single-writer.** Do these
  PRs SOLO — cold background agents stall at "compiles/green" without faithful/complete work
  ([[cold-agents-need-verification]]). Two agents both nudging the golden = unattributable drift.
- **Prove a spike before a big mechanism PR** (PR-D0 was the morphogen spike): a cheap throwaway test
  that the mechanism produces the phenomenon, gating go/no-go before production.

## 10. Running & inspecting

- Tests: **the gate is CI** (push → `bash scripts/ci-report.sh`, §4); local `./scripts/test-bar.sh` is
  optional for targeted iteration only, never bare `cargo test`.
- Headless: `cargo run -p animata-sim --bin headless --release`.
- Viewer + dev-bridge: `cargo run -p animata --features dev` (the dev-bridge port is PER-BRANCH —
  read it from `.animata-dev-port`, never assume 8127; see [[dev-bridge-port]]). Verify visual/render
  claims IN the running app, not by reasoning ([[verify-visual-fixes-in-app]]).
- Lint: `cargo clippy --all-targets --release` must be clean (warnings are errors via the kit gate).
- `.claude-dev-kit/**` is READ-ONLY here — fixes go upstream, never edit it locally.

## 11. The standard loop for a sim change

1. Read this skill + the relevant memory. Understand the determinism footprint of your change.
2. If it's a big mechanism: plan → plan-consensus → spike (gate) → implement.
3. Implement smallest-first; prefer inert-then-activate for risky determinism-critical work.
4. `cargo build -p animata-sim --release`; optionally run the ONE test you're iterating on locally
   (§4). The full-suite gate is CI, reached by pushing in step 8 — don't run it locally.
5. If the golden moved: confirm it was MEANT to, re-pin both profiles (§3) with a why-comment.
6. If a corridor broke: apply §5 (inert / RNG-preserve / multi-seed) — never silently weaken a test.
7. New field? Run the §7 checklist.
8. Branch (separate Bash call) → commit → **push → `bash scripts/ci-report.sh` (the gate: merge only
   on exit 0)** → subsystem-reviewer on the diff → fix FAILs → PR → merge → sync main → update memory.
