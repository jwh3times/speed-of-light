# Docs

## Doc map

- `README.md`: what Speed of Light is, how to download and run a release, the CLI's main commands,
  building from source, and "Help verify your GPU"; for users and contributors. Update when a
  `sol` or `sol-service` command or flag changes (`crates/sol-cli/src/main.rs`,
  `crates/sol-service/src/cli.rs`), what `install` sets up changes
  (`crates/sol-service/src/winsvc.rs`, `registration.rs`), when the release zip's contents change
  (`.github/workflows/release.yml`'s `package` job), or when a build step or the toolchain pin
  (`rust-toolchain.toml`) changes.
- `ROADMAP.md`: what is done and what comes next. Update its "Done" list when a feature lands;
  reordering or adding plans is the owner's call.
- `CONTRIBUTING.md`: how to contribute hardware reports and code, the checks a PR passes, the
  licence of contributions and the DCO sign-off. Update when the checks CI runs change
  (`.github/workflows/ci.yml`) or the contribution process does.
- `SECURITY.md`: how to report a vulnerability. `owner-only`.
- `CHANGELOG.md`: one dated section per version a merge mints, stamped by the release workflow's
  `scripts/stamp-changelog.mjs` after the merge (`docs/agents/releases.md`). `frozen`: no branch
  writes its sections.
- `changelog.d/README.md`: the fragment format and how fragments are stamped, for whoever writes
  one. Update when the fragment rules in `scripts/check-changelog.mjs` or the stamping in
  `scripts/stamp-changelog.mjs` change.
- `changelog.d/*.md`: each branch's changelog fragment (every file there but the README), stamped
  into `CHANGELOG.md` after its merge and checked by the `changelog` CI job. `ship-owned`, like the
  `VERSION` file.
- `docs/PRD.md`: goals, users and requirements, with the IDs (`FAN-3`) that reviews and issues
  cite. `owner-only`.
- `docs/architecture.md`: the process model, the data flow between CLI, service and backends, and
  why Rust. Update when a crate is added or its role changes (`Cargo.toml` members), or the
  service's process model, state folder or IPC transport does (`crates/sol-service/src/main.rs`,
  `host.rs`, `winsvc.rs`, `registration.rs`, `trust.rs`, `crates/sol-ipc/`).
- `docs/safety.md`: every safety invariant and the test that guards it. Update in the same change
  that adds or changes an invariant or renames its guarding test (`crates/sol-core/src/tuner.rs`,
  `fan.rs`, `crashguard.rs`, `crates/sol-service/tests/service.rs`, and the unit tests in
  `crates/sol-service/src/registration.rs`, `trust.rs` and `winsvc.rs`).
- `docs/testing.md`: the test layers (hermetic, simulation, live read-only probes, hardware
  reports) and how to add coverage. Update when a test layer, a fault in `sim::Faults` or a test
  command changes.
- `docs/compat-matrix.md`: the matrix format, precedence and how an entry gets its evidence.
  Update when `compat/matrix.toml`'s format or `crates/sol-core/src/compat.rs`'s rules change.
- `docs/vendor-apis.md`: NVML, NVAPI, ADLX, IGCL and PresentMon, what is used and what is
  missing. Update when a backend starts or stops using a vendor API (`crates/sol-nvidia/`,
  `sol-amd/`, `sol-intel/`).
- `docs/landscape.md`: competitors and the positioning this project takes. `owner-only`.
- `GLOSSARY.md`: the project's domain language (service, startup takeover, firmware control,
  crash guard, …), one tight definition per term with the words to avoid. No implementation
  detail. Update when a design session settles a term (`domain-modeling` skill).
- `docs/adr/*.md`: architecture decision records, one decision each with the reason it was made.
  Add one when a decision is hard to reverse and would surprise a reader; supersede rather than
  rewrite.
- `AGENTS.md`: guidance for every coding agent: what this is, commands, layout, the rules for every
  change, the area guides, the development flow and the agent skills. Update when a command, a crate
  or a rule changes, or `.claude/settings.json` does. The `shared:` blocks are written by
  `scripts/sync-agents.mjs`; never edit them here.
- `CLAUDE.md`: imports `AGENTS.md` for Claude Code. `owner-only`.
- `docs/agents/*.md`: settings the shared skills read. Update when this repository's process
  changes, or the `main` or tag rulesets do (`open-pr.md` lists the `main` rules, `merging.md`
  the tag rules).
- `docs/agents/merging.md`: from open PR to released and packaged version. Update when a merge or
  release lesson is learned that no check enforces, a command it quotes changes, or the `main` or
  tag rulesets do.
- `docs/agents/dependabot.md`: what reviewing a Dependabot pull request takes here, read by the
  `dependabot` skill. Update when `.github/dependabot.yml`, a workflow's toolchain or install steps,
  `rust-toolchain.toml` or the workspace's main dependencies change.
- `docs/agents/guard.md`: the commands `scripts/guard.mjs` denies or asks about here, and the
  paths it protects, on top of its universal rules. Update when a `sol` or `sol-service` command
  that writes to hardware is added or renamed, or a never-remove path in
  `docs/agents/workspace.md` changes.
- `docs/agents/test-proof.md`: how the `test-prover` skill runs one test on its own here. Update
  when a test folder, a crate or a test command changes.
- `.agents/skills/**`: installed skills, listed in `skills-lock.json`. `frozen`: a reinstall
  overwrites them, and `scripts/check-skills.mjs` fails on an edited copy.

## Drift probes

- CI jobs and their commands: `.github/workflows/ci.yml`.
- Required checks and the other `main` rules:
  `gh api repos/jwh3times/speed-of-light/rules/branches/main`.
- The tag ruleset's rules and bypass actors: `gh api repos/jwh3times/speed-of-light/rulesets/<id>`,
  the id from `gh api repos/jwh3times/speed-of-light/rulesets`.
- Crates in `AGENTS.md`'s layout and `docs/architecture.md`: the `members` of `Cargo.toml`.
- CLI commands in `README.md` and `AGENTS.md`: the `Command` enums in `crates/sol-cli/src/main.rs`
  and `crates/sol-service/src/cli.rs`.
- Guarding tests named in `docs/safety.md`: `fn <name>` under `crates/`.
- Release zip contents in `README.md`: the `package` job in `.github/workflows/release.yml`.
- The toolchain in `README.md` and `docs/agents/dependabot.md`: `rust-toolchain.toml`.
