### Added

- First code, licensed AGPL-3.0-only. See `ROADMAP.md` under "Done" for the feature list. Writes on
  real hardware are implemented but not yet verified on any driver, so they're blocked unless
  `--allow-unverified` is passed.
- Each release has a Windows x64 zip on its GitHub Release, holding `sol.exe`, `sol-service.exe`,
  the example profiles, the README, the license and the release notes, with a `SHA256SUMS.txt`
  beside it.
- The shared agent skills from `jwh3times/skills` and their settings under `docs/agents/`.

### Changed

- Every merge to `main` is a release: `.github/workflows/release.yml` stamps this changelog from
  `changelog.d/` fragments and tags the merge `v<major>.<minor>.<build>`. `VERSION` holds the
  release line, and `scripts/check-cargo-version.mjs` keeps Cargo's workspace version in step.
- The Rust toolchain is pinned to 1.88, the minimum supported version, in `rust-toolchain.toml`.
- CI runs `rust` (format, lint, test on Windows), `agents` (the repository scripts) and
  `changelog`, with every action pinned by commit.
