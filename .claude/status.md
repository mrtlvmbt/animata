task: #516 terragen-v3 Probe (PR #519, branch tg3-probe)
phase: code (ready for CI)
blocked_on: metric #5 fidelity still below 90% — hex grid coarseness (45 cells/hex) destroys fine drainage structure in smooth synthetic fields
next: CI run via git push + ci-report.sh; if gate fails, escalate to hex grid resolution (smaller HEX_GRID_SIZE) or pooling method (max height instead of mean)
updated: 2026-07-19 14:35
notes: COMPLETED: hex duplicate fix (axial distance generation), D6 accumulation height-descending sort, fallback flow to lowest neighbor, unit tests added. Improvements: DD Post +3-10x, valley relief now passes many combos, anti-spike passes both tiers. Gate rule unchanged (metric #4 + ≥1 combo #1/#2/#3/#5).
