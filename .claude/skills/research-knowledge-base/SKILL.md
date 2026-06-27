---
name: research-knowledge-base
description: >
  Build a rigorous, verified research/theory knowledge base of markdown docs (optionally to feed coder
  agents). Use to research a domain deeply, decompose a big topic into a structured doc set, write
  trustworthy design/research docs, run citation/web verification, take docs through
  cold-critic-to-consensus review, build live "calculate-on-shore" models, organize a multi-track doc
  tree with indexes.
triggers:
  - "research this"
  - "build a knowledge base"
  - "decompose into docs"
  - "doc per aspect"
  - "critique to consensus"
  - "verify citations / web research"
  - "feed coder agents"
---

# Research & Knowledge-Base Accumulation

A method for producing a **rich, accurate, self-contained** body of markdown docs that humans or downstream agents (e.g. coder agents) can trust and act on. Optimizes for: completeness, correctness, self-containment, navigability.

## When to use
- A large topic must be turned into a structured, durable doc set.
- Docs will brief other agents/humans, so errors are expensive → rigor required.
- The user asks to research, decompose, verify, critique-to-consensus, or organize a knowledge tree.

## Core principles
1. **Docs are artifacts, not chat.** Each is self-contained and survives independent reading.
2. **Trust is earned per-doc** via cold critique + web verification, not asserted.
3. **Quantitative claims get a live model** that actually runs and closes (see §5).
4. **Authorial judgment over critics.** Critics aren't infallible — verify disputed points (ideally via web) before "fixing"; record what you rejected and why.
5. **Report faithfully.** Never claim "verified / consensus / done" unless it actually happened. Distinguish "critic-checked" from "live-web-verified".

## Doc format (every doc)
`теория / theory → формализация/математика → выводы для <project> (design takeaways) → подводные камни (pitfalls) → ссылки (references)`. Keep terminology exact; match the surrounding docs' language and density.

## Structure: tracks + indexes
- Group docs into **parallel tracks** by the question they answer (e.g. "what" / "how to code" / "presentation" / "how much"). Each track is a directory with a `00-index.md`.
- Number docs within a track (`01`, `02`, …). Cross-link liberally between tracks.
- Maintain a **top-level master index** (e.g. `README.md`) as the entry point for downstream agents: purpose, the non-negotiable decisions/invariants ("the contract"), the track map, a reading order, provenance ("how this was verified"), and a status table.
- **Index integration is the orchestrator's job.** When spawning per-aspect agents, tell them NOT to edit the shared index (concurrent edits race) — collect and integrate indexes yourself after each lands.

## Per-aspect background agents (for breadth)
For a multi-aspect batch, spawn **one background agent per aspect**, each running the FULL cycle autonomously: read context → web research → write doc (+ model) → its own cold-critic-to-consensus → report. Launch in small waves (a couple at a time) to avoid overload. Give each agent:
- exact deliverable paths and "do not edit the shared index";
- the doc format + house conventions + the project's locked decisions;
- the scope constraint (what it may read; what is off-limits);
- the explicit instruction to ITERATE to consensus (not stop at "needs work"), and to have its critics **read files by absolute path, never pasted into the prompt** (pasting causes placeholder/empty-doc failures).

## Cold-critic-to-consensus protocol
Per file: spawn a **fresh** general-purpose subagent in **isolated context**, hostile / assume-wrong. It must READ the doc (and run the model) **by path**, and return exactly:
- `BLOCKING:` numbered must-fix (factual errors, hallucinated/mis-attributed citations, claims a model doesn't support, broken links, misleading claims that would cause wrong downstream work)
- `MINOR:` numbered nice-to-fix
- `VERDICT:` `CONSENSUS` (zero blocking) or `NEEDS-WORK`

Then: apply real fixes (judgment), spawn a **new** critic (not the same one), repeat until `CONSENSUS` with zero blocking. Typical: 1–5 rounds. Tell critics to judge by intended scope (a survey/design doc ≠ a full implementation spec) so they don't demand inappropriate detail.

## Live "calculate-on-shore" models
For quantitative docs, write a runnable script (e.g. python `@dataclass Params`, a `report()` that prints sections) that:
- derives equilibria/thresholds analytically AND verifies them by simulation;
- **closes its conservation/balance to ~machine epsilon** (a leaky balance = a modeling bug — fix it, don't relax the claim unless honestly justified);
- units may be abstract but must be internally consistent (ratios matter);
- put **only verified, run-produced numbers** into the doc; add a regression-test note.
Store scripts in **per-track `<track>/models/`** subdirs; docs link to `models/<name>.py`. Do the move as a FINAL pass (moving mid-flight races script-creating agents).

## Web verification
Run a **dedicated** web pass (not just critic citation-checks from model knowledge): verify every citation (author/year/title/venue), check post-cutoff currency of any tooling/ecosystem, and survey recent prior art. Record the verification date in the indexes. If a doc was only critic-checked, say so — don't conflate with live-web.

## Memory
Persist durable project facts to memory: the goal, the working protocol, the scope constraints, locked decisions, and the open-gaps map. Update as decisions change.

## Final QA (after all docs land)
1. **Link validation** — mechanically check every markdown link resolves (grep links → assert targets exist). Do this LAST (earlier runs false-positive on not-yet-created docs).
2. **Self-consistency** — scan for cross-doc contradictions (a claim in one doc vs another; shared invariants stated consistently).
3. **Models reorg** — move scripts into `<track>/models/`, fix links, confirm they still run.
4. Update the master index + status tables to final.

## Common pitfalls (seen in practice)
- A spawned agent **stops at "NEEDS-WORK"** instead of iterating → resume it (SendMessage preserves context) with the explicit blocking list and "iterate to consensus".
- A critic gets the doc **pasted as a placeholder** instead of reading it → instruct critics to read by path.
- **Hidden fitness / reward shaping** in simulation design = directed evolution, not the thing claimed → call it out as a design pitfall in relevant docs.
- **Over-pinning fast-moving versions** in docs → state currency + "fix at start", don't hard-pin.
- Claiming web-verified when only critic-checked → keep the distinction honest.
- Editing a shared index from parallel agents → races; integrate centrally.
