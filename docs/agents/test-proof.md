# Test proof

How `test-prover` runs one test on its own here, and how it tells a run that tested nothing from a
pass. The suites and their CI jobs are in `docs/agents/open-pr.md`; the test layers are in
`docs/testing.md`.

## Test commands

- repo root: Rust unit tests, in the `#[cfg(test)]` modules of `crates/<crate>/src/**/*.rs`.
  The crate is the second path segment of `<file>` (`crates/sol-core/src/fan.rs` → `sol-core`), and
  a test's name is its path inside the crate (`fan::tests::<fn>`). One file:
  `cargo test --locked -p "$(echo "<file>" | cut -d/ -f2)" --lib -- "$(basename "<file>" .rs)::"`
  (for `lib.rs`, drop the filter after `--`, which runs the crate's whole library suite); one test:
  `cargo test --locked -p "$(echo "<file>" | cut -d/ -f2)" --lib -- --exact "<name>"`. Count:
  cargo's `test result:` lines, which `mutate.mjs` reads already. Worktree: none; the first run
  builds the workspace into the worktree's own `target/`, about a minute.
- repo root: Rust unit tests in `crates/sol-cli/src/**/*.rs`. `sol-cli` builds only the `sol`
  binary, with no library, so the command names the binary instead of `--lib`. One file:
  `cargo test --locked -p sol-cli --bin sol -- "<module>::"` (for `main.rs`, `tests::`); one test:
  `cargo test --locked -p sol-cli --bin sol -- --exact "<name>"`. Worktree: as above.
- repo root: Rust integration tests, `crates/<crate>/tests/*.rs` (`sol-service`, `sol-cli`). One
  file: `cargo test --locked -p "$(echo "<file>" | cut -d/ -f2)" --test "$(basename "<file>" .rs)"`;
  one test: the same with `-- --exact "<name>"`, the name being the function's. `sol-cli`'s run
  the built `sol` binary (`CARGO_BIN_EXE_sol`) against a pipe name nothing serves, never the
  installed service's. Worktree: as above.
- repo root: `scripts/*.test.mjs`, node's runner. One file: `node --test <file>`; one test:
  `node --test --test-name-pattern="<name>" <file>`. Worktree: none (the scripts have no
  dependencies).
- Not proven: the live probes, `#[ignore]` tests in `crates/sol-nvidia`, need an NVIDIA GPU, and
  `test-prover` never runs them (they are read-only, but a scratch worktree is no place to reach
  real hardware).

## Guide

docs/testing.md
