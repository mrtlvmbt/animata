task: #523 (Slice-0: terragen-v3 parameter foundation)
phase: CI (pass 2 of 2 - golden verified, awaiting full suite)
blocked_on: v2 sim job (invariants + perf gate)
next: CI complete → post code-critic verdict → merge PR
updated: 2026-07-19 (current)

Golden-arm64 (byte-identity check): PASS ✓
compile-check: PASS ✓
tests (corridors, x86): PASS ✓
golden lock (arm64): PASS ✓
v2 golden (arm64): PASS ✓ (byte-identical goldens verified!)
v2 sim (x86): IN PROGRESS
