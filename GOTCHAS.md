# GOTCHAS — symptom-first index of rakes already stepped on

**Purpose:** when something breaks or behaves unexpectedly, look HERE FIRST before debugging from
scratch. Each entry is one wall someone already hit and solved, in the form **Symptom → Cause → What to
do**, concrete enough that the next session does not repeat it.

**Standing rule (all agents, all tasks):** after you hit a wall / unexpected behavior AND solve it,
append ONE compressed entry to the `## Gotchas` section below. Append-only — never rewrite or delete an
existing entry (unless it is proven wrong). Keep each entry to Symptom → Cause → What to do. This file is
the fast lookup; the long-form "why" stays in CLAUDE.md / memory / landmarks — link to them if useful.

## Gotchas

- **`rtk` cargo prints a SUMMARY saying "0 errors" but the build is actually broken** → the rtk proxy
  summary swallows/miscounts compiler output (observed: reported 0 while raw cargo had 21 errors) → never
  trust the rtk summary for a green verdict; re-verify with raw `rtk proxy cargo <cmd> … --message-format=short`.

- **`cargo build --release` is green but the crate is actually broken (call-sites fail)** → `cargo build`
  compiles the lib/bin only, NOT the test crate; a changed fn signature breaks call-sites that live in
  tests/unit modules and are invisible to `build` → compile via the TEST crate: `cargo test --no-run`
  (or `--all-targets`), never `cargo build`, to certify a signature change.

- **`gh`/`git` errors "fatal: not a git repository"** → the command was run from the PM reports dir or the
  wrong nesting level (`pm/` is a repo nested inside the `PM/` animata checkout) → run CODE gh/git from a
  code worktree (e.g. `/Users/spopov/projects/animata/A`); run REPORTS gh/git from `pm/` (animata-pm).

- **`gh pr merge --admin` is DENIED by the auto-mode classifier** → `--admin` bypasses required review/checks
  and needs EXPLICIT user authorization each time → try plain `gh pr merge --squash` FIRST (it goes through
  once required checks are satisfied); use `--admin` only after the user has explicitly sanctioned it.

- **A coder reports `STATUS: done` but the PR was never created / CI is not green** → coders fabricate
  "done" (documented pathology) → PM re-verifies with `.claude/hooks/done-check.sh` (needs open PR + green
  CI on HEAD + zero unchecked `- [ ]` acceptance items) and inspects the coder's `.claude/done-gate.log`
  for a `BLOCKED-OVERRIDE` / fresh `ALLOW` before accepting the handoff.

- **`cargo` in the repo root builds the wrong workspace / "manifest not found" for v2-sim symbols** → the
  repo root is the v1 workspace; the v2 simulation is a SEPARATE workspace under `v2/` → `cd v2` before any
  cargo command that targets v2-sim (a recurring coder trap).

- **`v2_golden_*` / `state_checksum` golden test falls locally on arm64 after a hash-contribution change**
  → goldens are arch-bound (the `golden-arm64` CI job on macos only; x86 excludes them, debug self-skips) →
  this is an expected re-pin, not a break; re-pin from `.ci-report/failed.log` (`golden-arm64` job), owner
  PM — do NOT re-pin from a local x86 run. (See the `crates/cli/tests/golden.rs` header.)

- **A memory file exists but the next session cannot find it** → it was written without a one-line pointer
  in `MEMORY.md`, so recall never surfaces it (observed: `reality-is-existence-proof-never-terminal` had a
  file but no index line) → after every memory Write, add the `MEMORY.md` index line in the same pass; when
  auditing, `grep` each memory filename against `MEMORY.md` to catch orphans.
