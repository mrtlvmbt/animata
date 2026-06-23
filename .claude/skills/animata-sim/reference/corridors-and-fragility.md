# Corridors & fragility — the #1 way to break things

**Read this when** a corridor test broke, or you fear a trajectory change will break one. This is
`SKILL.md` §5 in depth — §5 is the canonical rule; follow it.

## What a corridor is, and why it's brittle

Acceptance corridors (`camouflage_emerges`, `toxin_resistance_evolves`, `organs_emerge`,
`predation_emerges`, `multicellularity_emerges`, the seasonality/speciation ones) each run **ONE seed for
8000 ticks** and assert an emergent statistic clears a threshold. They are brittle to **ANY** trajectory
perturbation — even a change that merely *adds* genes shifts the mutation RNG stream and reshuffles which
single-seed corridor passes. Observed: a genome-widening change knocked `toxin` seed-1 to 0.009 while the
mean over seeds 1–5 was a healthy 0.113. **That is seed luck, not a broken mechanism.**

## The defence ladder (in order of preference)

1. **Make the change inert** (`determinism.md`, `SKILL.md` §6) — a byte-identical trajectory leaves every
   corridor untouched. Always the first choice for machinery.
2. **Preserve the RNG stream of existing genes.** When adding genome fields: draw the new fields LAST in
   `mutate` (struct-literal fields evaluate top-to-bottom) and as constants in `founder` (no rng), so the
   existing genes keep their exact draw sequence. This alone recovered one corridor from 0.032 → 0.188.
3. **Prove the mechanism survives across seeds, then make the corridor multi-seed robust** — probe seeds
   1–5 with a temporary `#[ignore]` test; assert the mean, or "≥k of n seeds clear the bar." A legitimate
   robustness fix, documented as such.

## The rule you may NOT break

**Never just lower a single-seed threshold to make your PR pass, and never cherry-pick a passing seed.**
That is hiding a regression, not fixing fragility. The multi-seed reframe (step 3) is only legitimate when
you have *shown the mechanism still fires* across seeds and the single seed was unlucky — the bar stays
strict, only the seed-aliasing is removed.

## Boom-bust: measure the phase, not the tick

The population oscillates hard (`measurement.md`). A single-tick `pop > N` corridor aliases the phase and
falsely reads "collapsed." The §5-correct fix is **phase-independent** metrics — peak > threshold over the
run, never-extinct, max-mechanism-strength over the run, multi-seed — NOT a weakened bar. Always probe the
pop trajectory (checkpoints to 8000t) before declaring a corridor break a regression.

## After an intended trajectory change

Re-pin both profile goldens (`determinism.md`, `SKILL.md` §3) AND do this corridor work — the re-pin and
the §5 robustness pass are the *same job*, routine cost for an intended change (memory
`golden-repin-is-fine-for-intended-change`), not a deterrent. Then a full `--release`
`./scripts/test-bar.sh` must be green, and a `subsystem-reviewer` pass is mandatory (pass it
`reference/determinism.md` + the diff).
