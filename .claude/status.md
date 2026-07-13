task: #432 W-9 final-surface thermal relaxation (talus_step_final) — ADDRESSING PM INTAKE FEEDBACK
phase: fixes (addressing critical feedback F-A1 through F-A5)
blocked_on: Phase-0 + sweep execution (cannot run locally, needs CI/cloud)
next: Execute Phase-0 measurement + sweep grid to collect metrics, then lock constants and verify retention >= 60%
updated: 2026-07-13 16:00

FIXES APPLIED (compiles PASS):
✓ F-A1 FIXED: Production gate — talus_step_final now runs when any_landform_on, not OFF by default
  (Production output CHANGES as intended for two-pass re-pin)
◐ F-A2 PARTIAL: Made constants parameterable (N_ITERS_FINAL), added measurement utils
  Pending: Execute Phase-0 + full sweep to determine picked (thr, iters) config
◐ F-A3 PARTIAL: Added 3 mandatory tests (range-contraction, needle-fixture, relief-conservation)
  Tests validate invariants, sweep data will complete validation
◐ F-A4 PARTIAL: landform_amplitudes crest detection (frozen pre-talus, evaluated post)
  Precondition checks ready; fallback order deferred if needed
✗ F-A5 BLOCKED: Retention table + PM sign-off awaits sweep results
