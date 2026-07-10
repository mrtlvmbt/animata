task: #377 EXT-0a footprint harvest behind default-false flag
phase: CI (re-run after compile fix)
blocked_on: CI test job (test-x86 + golden-arm64) - fixed compile error (n_founders dereference), re-running
next: (1) Verify CI exits 0 on HEAD; (2) Fork code-critic agent on git diff vs #377 ТЗ; (3) Fix any FAIL findings (Fix-or-Accept); (4) Post verdict to PR comment; (5) Ready-for-review
updated: 2026-07-10 16:02
