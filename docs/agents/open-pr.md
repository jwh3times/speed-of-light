# Open PR

## Fast checks

- Repo root: `cargo fmt --all --check`, only when `crates/` or `*.rs` changed, fixed by
  `cargo fmt --all`.
- Repo root: `cargo clippy --workspace --all-targets --locked -- -D warnings`, only when `crates/`,
  `Cargo.toml`, `Cargo.lock`, `clippy.toml` or `rust-toolchain.toml` changed.
- Repo root: `cargo test --workspace --locked`, only when `crates/`, `compat/`, `Cargo.toml`,
  `Cargo.lock` or `rust-toolchain.toml` changed (hermetic: the simulator, never a GPU).
- Repo root: `node scripts/sync-agents.mjs --check`, fixed by `node scripts/sync-agents.mjs`,
  committing both sides.
- Repo root: `node scripts/check-skills.mjs`, fixed by re-running the install commands in the
  `shared-skills` block of `AGENTS.md`.
- Repo root: `node scripts/lint-agent-neutrality.mjs`.
- Repo root: `node scripts/check-docs.mjs`.
- Repo root: `node scripts/check-cargo-version.mjs`, fixed by
  `node scripts/check-cargo-version.mjs --set <VERSION's value> && cargo update --workspace`.
- Repo root: `node --test "scripts/*.test.mjs"`, only when `scripts/` changed.

CI runs the `cargo` entries in its `rust` job on `windows-latest`, and every `node` entry above in
its `agents` job on every run, the script tests included. The `changelog` job runs the `Changelog
check` on every pull request, Dependabot's included. Nothing is CI-only: the live probes
(`cargo test -p sol-nvidia -- --ignored`) need a GPU and run in neither.

The `Main branch rules` ruleset on `main` requires a PR (merge commits only, review threads
resolved), the `rust`, `agents` and `changelog` checks passing on a branch up to date with `main`,
and CodeQL code scanning (`gh api repos/jwh3times/speed-of-light/rules/branches/main`).
