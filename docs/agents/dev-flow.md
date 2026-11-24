# Development flow

The flow itself is `.agents/skills/open-pr/settings/dev-flow.md`, and `AGENTS.md` summarizes it.
This file holds what this repository adds.

## Hardware writes are never part of a phase

`AGENTS.md` rule 5 holds inside every phase: an agent never runs `sol selftest --write`,
`sol apply`, `sol reset`, `sol fan` or a real (non-`--simulate`) service against the user's GPU
without asking first, and `scripts/guard.mjs` asks before each (`docs/agents/guard.md`). A change
that needs evidence from real hardware records that run as a human follow-up
(`docs/agents/human-follow-up.md`).

## After the pull request opens

`/open-pr` stops once the PR is open. Merging and its follow-ups are in `docs/agents/merging.md`.
