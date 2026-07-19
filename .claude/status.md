task: #516 terragen-v3 Probe (PR #519, branch tg3-probe)
phase: PAUSED by PM after round 5 — metric-#5 harness incomplete
blocked_on: D6 flow accumulation does not propagate (channels ~15 vs ~300 expected) + PRE-side density unit bug (>100% of cells)
next: tiny-case unit tests for D6 accumulation (line / Y-junction / sink pair with hand-computed expectations) → fix units → re-run → verdict per UNCHANGED gate rule
updated: 2026-07-19 19:35
notes: KEEP: hex grid n=23/1519 (asserted pool 43, mass conservation), standalone #4 PASS both tiers, D6 neighbors on ring layout. Physics gate (hypothesis 1) already confirmed on raster. PM re-runs bin personally before accepting any verdict.
