task: PR #437 — R-15a retained GPU buffers parity fix
phase: COMPLETE
blocked_on: none
next: PM merge review
updated: 2026-07-14 10:05

COMPLETION SUMMARY:
- Parity check: PASS (cmp -s exit 0, byte-identical PNGs)
- Compile check: PASS (scripts/compile-check.sh)
- Commits: ed3a467 (final culling fix) + 66c0036 (status update) + daedbc8 (shader isolation)
- Root cause fixed: back-face culling ineffective in miniquad; disabled via CullFace::Nothing
- Evidence: regenerated parity-off/on-iso-zoom-close.png (both 881K, identical)
