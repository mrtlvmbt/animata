task: W-11: ridged mountain belts (incl. W-0 LandformFlags struct) - #466
phase: PM audit fixes applied + PR live + CI running
blocked_on: CI completion (pass 2 of 2)
next: CI result → merge if PASS
updated: 2026-07-17 15:10

## Progress
- Critic fixes applied (F1/F2/F3) — Re-critic PASS ✓
- PM audit fixes applied:
  * Added w11_chain.rs tests (flag-off purity, clamp/bounds, salt-independence)
  * RIDGE_AMP candidates exposed (15/10, 25/10, 40/10) with ACTIVE_RIDGE_AMP_INDEX
  * Clarified const-assert comment (coupling formula ambiguous, pending PM)
- Commit cb47c53 pushed, PR #467 body updated with candidate details
- Compilation: PASS
- CI running on cb47c53
