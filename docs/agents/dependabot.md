# Dependabot

What reviewing a Dependabot pull request takes here, read by the `dependabot` skill in a session
and in the scheduled review. These facts apply to this repository only.

## Stack

- **The Rust workspace** (`Cargo.toml`, `Cargo.lock`, `crates/*`): a Windows GPU tuning tool.
  `nvml-wrapper` binds NVIDIA's NVML (`sol-nvidia`); `windows-sys` covers the named pipe, its DACL
  (`sol-ipc`) and the process scan (`sol-service`); `tokio` runs the service and its pipe; `clap`
  parses the CLI; `serde`, `serde_json` and `toml` carry the IPC wire format, the profiles and the
  compiled-in compat matrix; `thiserror` and `anyhow` the errors; `tracing` the logs; `proptest`
  and `tempfile` are test-only. `nvml-wrapper-sys` and `libloading` are declared in
  `[workspace.dependencies]` for the backends to come, but no crate uses them yet, so their bumps
  only move `Cargo.toml`.
- **Toolchain:** `rust-toolchain.toml` pins Rust 1.88, the same as `rust-version` in Cargo.toml (the
  MSRV). Dependabot does not bump it.
- **Node 22** (`.nvmrc`) runs the repository scripts, which have no dependencies and no
  `package.json`.

`.github/workflows/ci.yml` runs three jobs, all required on `main`: `rust` (on `windows-latest`:
`rustup toolchain install`, then `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets --locked -- -D warnings` and `cargo test --workspace --locked`), `changelog`
(`node scripts/check-changelog.mjs`) and `agents` (the repository scripts on Node 22).
`release.yml` runs on a push to `main`: `tag` stamps the changelog and tags the merge, and
`package` builds the release zip on Windows.

`.github/dependabot.yml` opens updates daily; one review covers a grouped PR:

- **cargo, `/`:** `cargo-minor-and-patch` groups minor and patch updates; each major is its own PR.
- **github-actions, `/`:** `actions-minor-and-patch`; each major is its own PR. Actions are pinned
  by commit SHA with a version comment, and Dependabot updates both.

## Review triggers

- **`nvml-wrapper`, on any bump.** It binds the driver's NVML ABI, and a change in how it maps an
  NVML call or error reaches real hardware, which no CI job has. The
  tests use fakes, so a green CI does not show the mapping is still right.
- **`windows-sys`, on any bump.** The pipe's security descriptor (`crates/sol-ipc/src/pipe.rs`)
  depends on its types and feature names; a minor can rename a feature the workspace enables.
- **A dependency whose new version raises its own `rust-version` above 1.88.** CI builds on 1.88,
  so it goes red; that is the MSRV holding, not a flake (Grounding facts).

## Resolution

- **Toolchain.** `rustup toolchain install` at the repository root installs the 1.88 toolchain
  `rust-toolchain.toml` names, with clippy and rustfmt. Node 22 from `.nvmrc`.
- **cargo** (`Cargo.toml`, `Cargo.lock`), from the repository root:
  `cargo build --workspace --locked`.

## Where to look

- `Cargo.toml`'s `[workspace.dependencies]` (versions and enabled features, the `windows-sys`
  feature list above all) and `[workspace.lints]`.
- `crates/sol-nvidia/src/lib.rs` (nvml-wrapper), `crates/sol-ipc/src/pipe.rs` and
  `crates/sol-service/src/conflicts.rs` (windows-sys), `crates/sol-ipc/src/protocol.rs` (serde, the
  pinned wire format), `crates/sol-core/src/profile.rs` and `compat.rs` (toml),
  `crates/sol-cli/src/main.rs` and
  `crates/sol-service/src/main.rs` (clap, tokio).
- `clippy.toml` and `rustfmt.toml`, for a toolchain-adjacent change.
- The `with:` inputs in `ci.yml` and `release.yml`.

## Grounding facts

- **CI builds on Rust 1.88, the MSRV.** A dependency that needs a newer compiler fails the `rust`
  job with an error naming the required `rustc` version. The fix is a decision, not a retry: hold
  the dependency back, or raise `rust-version` and `rust-toolchain.toml` together in their own PR.
- **The IPC wire format is a contract** (`AGENTS.md` rule 7): `wire_format_is_stable` in `sol-ipc`
  pins the JSON. A `serde` or `serde_json` bump that changes serialized output fails that test, and
  the fix is never to update the pin in a dependency PR.
- **No test touches a GPU.** The hermetic suite uses `sol_core::sim::SimulatedGpu` and fakes; the
  live probes are `#[ignore]`d and run in no CI job. A green CI says nothing about NVML on real
  hardware (Review triggers).
- **The changelog check says nothing about the dependency.** CI runs `check-changelog.mjs` with no
  `--source`, so a branch with no `changelog.d/` fragment, as a Dependabot branch is, passes it. At
  release, `scripts/stamp-changelog.mjs` writes that merge's entry under `Changed` from its PR title.
- **Clippy warnings are errors.** A clippy that comes with a toolchain change can add lints; a
  dependency bump alone does not change clippy, since the toolchain is pinned.

## Playbook

- **`nvml-wrapper` major:** read its changelog for every major crossed, then search
  `crates/sol-nvidia/src/` for each changed API. Gates: the Resolution build, `cargo clippy` and
  `cargo test --workspace --locked`. Ask the maintainer to run the read-only live probe
  (`cargo test -p sol-nvidia -- --ignored --nocapture`) on their GPU before merging.
- **GitHub Actions major:** NEEDS EFFORT only when a removed or renamed input, or a changed default,
  reaches a `with:` input; name the step. The inputs in use: `actions/checkout` (`fetch-depth: 0`,
  `ref`, `token`, `persist-credentials: false`), `actions/setup-node` (`node-version-file: .nvmrc`),
  `actions/create-github-app-token` (`client-id`, `private-key`, `permission-contents: write`) and
  `Swatinem/rust-cache` (none).
