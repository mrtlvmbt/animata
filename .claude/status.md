task: TOPO-DIFF Rung-0 probe validity gate (stand-down: fatal capability flaw caught pre-dispatch)
phase: blocked (do NOT dispatch, do NOT merge)
blocked_on: direction (pivot back to user — economy has no germ marginal return structure)
next: user decision on differentiation strategy (economy is monotone-decreasing in germ by construction)
updated: 2026-07-11 14:55 (validity gate check 1 failure confirmed by PM + adversarial reviewer)

## VALIDITY GATE FAILURE: Check 1 (Capability)

**Finding (PM + adversarial reviewer, 2026-07-11 14:55):**
Under `fate_economy`, germ has ZERO positive marginal return in EVERY resource regime:
- Income: monotone in soma (deficit saturation makes soma concave but never reverses it)
- Reproduction: binary germ>0 gate (no fecundity modulation)
- Death: energy-driven (germ-heavy bodies starve MORE, not less)
- Predation: body-size driven

**Consequence:** germ:soma fitness curve is monotone-decreasing in germ
- Maximum always at germ=1 (lowest fertile point)
- Interior PEAK impossible by construction
- Probe could not fire; running would waste computational resources on foreordained NULL

**My Error:**
❌ Marked check 1 "✅ Capability: interior split CAN beat both extremes under deficit saturation"
- Identified deficit saturation makes soma's return concave (TRUE)
- Missed: germ has ZERO positive return to pair with it (FALSE PREMISE)
- Confusion: Concave soma ≠ interior optimum; need BOTH arms to have positive returns

**Lesson Internalized:**
"Capability" = naming the mechanism by which TREATMENT WINS, not where baseline weakens.
Interior optimum requires DUAL leverage: both germ AND soma must have positive returns somewhere.
One concave curve + one zero-return curve = monotone, not optimum.

**Status of Deliverables (left as-is, NOT merged):**
- ✅ topo_diff_rung0_multientity_deficit_probe.rs (valid test scaffold)
- ✅ topo-rung0-deficit-probe-preregistration.md (honest analysis; shows fatal flaw)
- ✅ PR #391 (open, unmerged; gate comment visible to PM)

**Outcome:**
Validity gate did its job — caught fatal flaw PRE-DISPATCH, not after a wasted run.
Pivot direction returns to user. No further action until user decides on differentiation strategy.
