# Measurement — the benchmarking iron-rules

**Read this when** you are about to benchmark a sim change. These rules exist because each one has
already produced a FALSE result that cost real time. Obey them before trusting any number.

## The two most-violated gotchas — internalise these first

1. **Interleaved A/B only. A single cold baseline LIES.** Build BOTH binaries, then alternate runs
   (base, feat, base, feat, …). A non-interleaved before/after once showed a fake 20% decide "win" that
   the interleaved A/B revealed as pure measurement artifact (decide was compute-bound and untouched).
   Never trust a cold before/after.
2. **The rtk-proxy stale-binary trap.** `rtk proxy cargo test` (and a rtk-wrapped `cargo run`) can serve
   a STALE cached binary after a rebuild — it will show pre-change numbers and look "bit-identical-
   impossible." For a fresh A/B, run the **test/bench binary DIRECTLY**
   (`target/release/deps/animata_sim-<hash> <filter> --nocapture`, or the freshly-built `headless`),
   never through the proxy. (Run tests via `./scripts/test-bar.sh` — it bypasses the proxy too.)

## Isolation — a shared/throttled box inflates everything

- **Run the bench alone.** A `--bench-pop 200000` run while tests or another rayon-saturated bench run
  inflates the tick 2–3× (once 10–20×, with garbage sign-flipping deltas). Wait for a quiet window.
- **Thermal throttle** after sustained 200k benches silently inflates later runs — a clean 5/5 needs a
  cool, idle machine.
- **macOS load anomaly:** load average ~30 with idle CPU is NOT memory pressure — macOS counts
  blocked/IO-wait threads. Don't gate a bench on `uptime` load (a `<6` gate hangs forever on a box with
  chronic load~30 and a calm CPU). Judge by actual CPU/thermal state, not the load number.

## The population is a BOOM-BUST oscillator — never read a single tick

The herbivore population swings wildly across ticks on the same seed (e.g. 48 → 23030 → 1205 over a few
thousand ticks). A single-tick metric (`pop > 100` at tick T) **aliases the phase** and falsely reads
"collapsed." So:

- For perf, average several rounds; a single tick's ms is noise.
- For corridors, use **phase-independent** metrics (peak over the run, never-extinct, max-mechanism),
  multi-seed — see `corridors-and-fragility.md`. Always probe the pop TRAJECTORY (checkpoints to 8000t)
  before calling a corridor break a regression.

## Verify a "byte-identical" claim, don't assume it

A structural change that should be inert must keep `state_checksum` (`sim.rs:1523`) equal OFF vs ON. The
authoritative check is the CI gate (`git push` → `bash scripts/ci-report.sh` exit 0 — the `golden-arm64`
job runs the release golden locks; `SKILL.md` §4). For fast local iteration on a spike, a TARGETED
`./scripts/test-bar.sh -p animata-sim state_checksum_replays_to_golden` is the allowed optional run (not
the gate). If the golden moved, the relayout leaked a reorder/round-trip bug; FIX it, do not re-pin
(`determinism.md`).
