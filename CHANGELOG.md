# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) via the
`v<major>.<minor>.<build>` tags minted by `.github/workflows/release.yml`. Branches never edit this
file: each writes its entries to `changelog.d/`, and the workflow stamps them into the section for
the version its merge mints ([ADR 0001](docs/adr/0001-stamp-changelog-on-tag.md)).

_`v0.0.0` marks the initial commit, which predates this scheme and has no section; its code is
released in 0.1.0._

## [Unreleased]

No unreleased changes.

## [0.1.7] - 2026-11-26

### Changed

- Take the latest shared skills and scripts from `jwh3times/skills` (#12)

## [0.1.6] - 2026-11-26

### Changed

- The release settings ignore the `v0.0.0` tag, which marks the initial commit and has no changelog
  section, so the records audit no longer reports it as released without one (#9)

## [0.1.5] - 2026-11-25

### Changed

- Take the latest shared skills and scripts from `jwh3times/skills` (#10)

## [0.1.4] - 2026-11-25

### Changed

- The agent docs record that merges never edit `VERSION`, the `main` and `Version tags` rulesets in
  full, and that a pull request with no CodeQL analysis on its head cannot merge until a new commit
  starts one (#5)

## [0.1.3] - 2026-11-25

### Changed

- Take the latest shared skills and scripts from `jwh3times/skills` (#7)

## [0.1.2] - 2026-11-24

### Changed

- Take the latest shared skills and scripts from `jwh3times/skills` (#6)

## [0.1.1] - 2026-11-24

### Changed

- build(deps): Bump `toml` from `0.9.12+spec-1.1.0` to `1.2.2+spec-1.1.0` (#4)

## [0.1.0] - 2026-11-24

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
