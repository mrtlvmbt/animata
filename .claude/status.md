task: W-11: ridged mountain belts (incl. W-0 LandformFlags struct) - #466
phase: code (critic fixes applied) + PR ready
blocked_on: PR creation + CI
next: create PR → run CI → await PASS → merge
updated: 2026-07-17 15:00

## Progress
Critic found 3 bugs (F1/F2/F3) in flags usage. Fixed:
- F1: Use LandformFlags::new() with enable parameters + clamps (lib.rs)
- F2: Widen de_needle_pass gate to all 7 flags (caps.rs:862)
- F3: Remove redundant flag condition from talus_step_final (caps.rs:853)
Commits 0b75a2a pushed. Compilation: PASS.
