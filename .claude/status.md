task: #390 TOPO-DIFF Rung 0 — fate-keyed germ/soma economy + IMPOSED-SPLIT verdict probe
phase: code (implementation + compile-only tests green; cloud CI verdict run pending)
blocked_on: CI verdict run (topo_diff_rung0_imposed_split_verdict via sim-run scenario, measures fitness curve across germ:soma sweeps)
next: (1) Run bash scripts/ci-report.sh to verify x86 corridors + golden byte-identity when fate_economy=false; (2) Fork code-critic agent on git diff vs #390 ТЗ (MANDATORY, pre-review); (3) Fix any FAIL findings; (4) Post verdict to PR; (5) Dispatch cloud probe (sim-run or CI), measure interior-max fitness curve, report PASS/NULL
updated: 2026-07-11 01:15
