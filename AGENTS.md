# AGENTS.md

Guidance for coding agents (and humans) working in this repository. It holds what applies to
**every** change. Area detail lives in `docs/`, linked below. Read the relevant one before
changing that area.

## What this is

Speed of Light is a Windows GPU tuning / fan control / monitoring tool written in Rust, aiming
to replace MSI Afterburner as the community default. Its differentiator is safety and trust:
writes are gated by a driver compatibility matrix, read back and verified, and fan control
fails towards cooling. See `README.md` for the pitch, `docs/PRD.md` for requirements (IDs like `FAN-3` are
referenced in code reviews and issues), and `ROADMAP.md` for what's next. Licensed
AGPL-3.0-only. Don't add dependencies with incompatible licenses (MIT, Apache-2.0, BSD, MPL-2.0
and LGPL are fine).

## Commands

```powershell
cargo build --workspace
cargo test --workspace                              # hermetic: simulator only, no GPU needed
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all                                     # CI runs `cargo fmt --all --check`

cargo test -p sol-nvidia -- --ignored --nocapture   # live READ-ONLY probe, needs an NVIDIA GPU
cargo run -p sol-cli -- probe                       # read-only, real hardware
cargo run -p sol-service -- run --simulate 1        # service on simulated GPUs
cargo run -p sol-cli -- status                      # talk to it
```

In Git Bash, backslashes in `\\.\pipe\...` get mangled. Pass a custom `--pipe` from PowerShell,
or leave the default.

The repository scripts need only Node 22 (`.nvmrc`), with nothing to install. CI's `agents` and
`changelog` jobs run them:

```bash
node scripts/sync-agents.mjs --check     # agent config is in sync
node scripts/check-skills.mjs            # installed skills match skills-lock.json
node scripts/lint-agent-neutrality.mjs   # skill and agent sources read correctly from both trees
node scripts/check-docs.mjs              # doc links resolve and docs/agents/docs.md maps every doc
node scripts/check-cargo-version.mjs     # Cargo.toml's workspace version matches VERSION
node scripts/next-version.mjs            # the version the next merge to main will mint
node scripts/check-changelog.mjs         # the branch's changelog.d/ fragment is valid
node scripts/stamp-changelog.mjs --dry-run   # the sections the release workflow would stamp
node --test "scripts/*.test.mjs"         # tests for the scripts above
```

Once a PR is open, read [docs/agents/merging.md](docs/agents/merging.md).

## Layout

- `crates/sol-core`: everything that decides *whether* and *how* to write. No hardware.
  `tuner` (plan / gate / write / verify), `fan` (curve engine), `profile` (TOML),
  `compat` (matrix), `crashguard`, `sim` (simulated GPU with fault injection).
- `crates/sol-nvidia`: NVML backend. `sol-amd` / `sol-intel`: detection stubs.
- `crates/sol-ipc`: protocol types, JSON-lines framing, named pipe with a restrictive DACL.
- `crates/sol-service`: `Service` (sync, transport-free, tested in `tests/service.rs`) plus
  `host` (tokio, pipe), shared by `run` in a console and the Windows service in `winsvc`
  (`install` / `uninstall` and what the SCM starts). Also `cli` (the `sol-service` command line),
  `registration` (the service's registration as data), `trust` (the state folder's ACL),
  `backends` (open all vendors) and `conflicts`.
- `crates/sol-cli`: `sol`. Service-backed commands plus direct `probe` / `doctor` / `selftest`.
- `compat/matrix.toml`: compiled into `sol-core`. Entries need evidence.

## Rules for every change

1. **All hardware writes go through `sol_core::tuner`** (`execute` or `reset`). The service's
   fan loop is the one exception: it writes duties computed by `FanController` and clamped by
   `FanCaps`. `sol selftest` uses `tuner::write_and_verify` directly because it *produces* the
   matrix evidence, and it must always restore. Don't add another write path.
2. **Backends are thin.** They translate calls and errors. No range checks, policy or retries
   in `sol-nvidia` and friends: those belong in core, where the simulator can test them.
3. **Missing is `None`, never zero.** A sensor the driver doesn't expose is `None` in
   `Telemetry` / `Capabilities`.
4. **Fail towards cooling.** Any new fan path must hand back to firmware on sensor loss, write
   failure and shutdown. Any clamp on a duty must only ever raise it. See `docs/safety.md`.
   Each invariant there names the test that guards it. Keep those tests passing, and add one
   for each new invariant.
5. **Tests never write to real hardware.** Live tests are `#[ignore]` and read-only. Anything
   that writes uses `sol_core::sim::SimulatedGpu`. Never run `sol selftest --write`, `sol apply`
   or a real (non-`--simulate`) service against the user's GPU without asking them first,
   especially while other tuning tools are running. The same goes for `sol-service install` and
   `uninstall`, even with `--simulate`: they register or remove a service on the user's machine,
   and uninstalling a real (non-`--simulate`) install resets every GPU to stock.
6. **Compat matrix entries need evidence**: a hardware-report issue link or `selftest` output.
   Never add a `verified` entry you didn't observe. The build rejects entries with empty
   evidence.
7. **Wire format is a contract.** `sol-ipc` request/response JSON is used by external clients.
   Changing it means bumping `PROTOCOL_VERSION` and updating `wire_format_is_stable`.
   The profile `schema` follows the same rule (`profile::SCHEMA_VERSION`).
8. `unsafe` only in FFI and Win32 code, each block with a `// SAFETY:` comment.
9. `unwrap` / `expect` are lint warnings (errors in CI) outside tests.

## Area guides

- [Architecture](docs/architecture.md): process model, data flow, why Rust.
- [Safety](docs/safety.md): invariants and the tests that guard them.
- [Testing](docs/testing.md): the test layers and how to add hardware coverage.
- [Compatibility matrix](docs/compat-matrix.md): format, precedence, how entries get added.
- [Vendor APIs](docs/vendor-apis.md): NVML / NVAPI / ADLX / IGCL / PresentMon, what's used
  and what's missing.
- [Landscape](docs/landscape.md): competitors and the positioning this project takes.

## Development flow

<!-- shared:development-flow -->

The flow skills are this repository's binding process for changing code. Read
`.agents/skills/open-pr/settings/dev-flow.md` (the routing, exemptions and issue linking) and
`docs/agents/dev-flow.md` (what this repository adds) before the first edit of a change, and whenever
no ready issue exists. `/ask-matt` maps the flow itself.

- **The gate.** Every code edit starts from a ready issue: an open `ready-for-agent` issue whose body
  is a `/to-spec` spec or a `/to-tickets` ticket or that carries a `/triage` brief, or a `bug` issue
  worked through `/diagnosing-bugs`, which needs the number and no spec. With none, stop before the
  first edit and route the user. Reading code, running tests and answering questions need no issue.
- **Private work.** A ready issue in the private tracker counts. Its number stays out of the branch,
  the PR, commits and public issues, and it is closed by hand after the merge.
- **The human starts every phase.** `/wayfinder`, `/grill-with-docs`, `/to-spec`, `/to-tickets`,
  `/implement` and `/triage` stay user-invoked. At a phase boundary, name the next command, say in
  one line why, and wait. Inside a phase, use `/tdd`, `/code-review`, `/diagnosing-bugs`,
  `/domain-modeling`, `/codebase-design`, `/prototype` and `/research` whenever they fit.
- **Exempt from the gate:** dependency bumps, changes touching only docs and agent config, release
  prep, and an explicit user waiver recorded in the PR body as `Flow waived by the user: <reason>`.
- **Linking.** Branches are `<type>/<issue#>-<slug>`; the PR body opens with `Closes #N` or
  `Part of #N`.
- **The flow ends at `/open-pr`.** `/implement` stops at a commit; `/open-pr` refreshes the docs,
  writes the changelog and the PR body, and opens the PR (Claude Code's built-in `/ship` is a
  different command, outside this flow). When the user says to merge it, `/land` merges it
  and confirms its release. `/dependabot` reviews the open Dependabot PRs and hands the ones the
  user names to `/land`; nothing merges a Dependabot PR on its own. `/ask-matt` also routes to
  `/pr`, `/retro` and `/implement-spec`, which are not installed here.
- **Tests are proven, reds are read.** Before `/implement` commits a change that adds or changes
  tests, and in `/code-review`, run `/test-prover`: in a scratch worktree it reverts the fix and
  breaks its guards, and every new test must go red. `/open-pr` runs it too. `/land` and
  `/dependabot` read a red check with `/ci-triage`, which never re-runs anything.
  `/records-audit` checks the board, the trackers, the follow-up wiki and the changelog against
  each other; run it weekly, and `/what-next` says when it is due.
- **`/handoff` is the shared skill.** It parks the session for another machine or harness, closes it
  out with `/end-session`, and `/lets-go` resumes it. It does not write the portable file `/ask-matt`
  describes, so a `/prototype` detour stays in the session or uses a branch.
- **A wizard is not the record.** A step only a human can take follows the `## Procedure` in
  `.agents/skills/open-pr/settings/human-follow-up.md`, whose instructions stand without the script.
- **Research goes where it is safe to keep.** `/research` findings that would expose private material
  go to the private companion, where there is one, never to an ignored folder with no repository
  behind it. Details of an unpublished vulnerability are a draft advisory, never a research file.

<!-- /shared:development-flow -->

## Agent skills

Skills are authored in `.agents/skills/` (Codex reads it directly); `.claude/skills/` is a
generated mirror for Claude Code. Never edit the mirror: run `node scripts/sync-agents.mjs`.

### Shared skills

<!-- shared:shared-skills -->

`end-session`, `handoff`, `lets-go`, `open-pr`, `land`, `dependabot`, `docs-updater`, `what-next`,
`test-prover`, `ci-triage`, `records-audit` and `tldr` install from `jwh3times/skills`, and are
never edited here: reinstalling overwrites a local edit.
A fix goes upstream, and what they need to know about this repository goes in the settings files
under `docs/agents/`, which each skill proposes on first run.

The other shared skills come from one `mattpocock/skills` snapshot (v1.3.2, pinned by commit), the
same in every repository, and are never edited here either: what they need to know goes in this
file or `docs/agents/`. Moving to a newer snapshot is a change in `jwh3times/skills`. Keep
`.agents/skills/` out of every formatter: `scripts/check-skills.mjs` fails when an installed copy
differs from `skills-lock.json`, or the lock from these commands.

Re-running the install is the update; never run `npx skills update`, which leaves `.claude/skills/`
linked to its source instead of copied:

```bash
npx skills add jwh3times/skills -s end-session -s handoff -s lets-go -s open-pr -s land -s dependabot -s docs-updater -s what-next -s test-prover -s ci-triage -s records-audit -s tldr -a codex --copy -y
npx skills add mattpocock/skills#d1478fcb140db0b93c0c8df17bcfd311844749d0 \
  -s ask-matt -s code-review -s codebase-design -s diagnosing-bugs -s domain-modeling -s grill-me \
  -s grill-with-docs -s grilling -s implement -s improve-codebase-architecture -s prototype \
  -s research -s setup-matt-pocock-skills -s tdd -s teach -s to-questionnaire -s to-spec \
  -s to-tickets -s triage -s wait-what -s wayfinder -s wizard -s writing-for-agents \
  -a codex --copy -y
node scripts/sync-agents.mjs
```

`scripts/sync-agents.mjs`, `lint-agent-neutrality.mjs`, `check-docs.mjs`, `check-skills.mjs`,
`guard.mjs`, and `sync-main.mjs`, `next-version.mjs`, `check-changelog.mjs` and
`stamp-changelog.mjs` where this repository carries them, are canonical in `jwh3times/skills` with
their tests: fix them there, then copy them here.
`sync-agents.mjs` also writes the marked blocks in this file: change them in `jwh3times/skills`, not
here. It registers `guard.mjs` in `.claude/settings.json`, which stops the commands and edits these
rules forbid; this repository's own additions go in `docs/agents/guard.md`.

<!-- /shared:shared-skills -->
