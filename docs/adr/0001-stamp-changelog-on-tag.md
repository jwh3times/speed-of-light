# Each merge's changelog section is stamped after it lands

Every merge to `main` is a release. A branch writes its changelog entries to its own
`changelog.d/<branch-slug>.md` fragment and never edits `CHANGELOG.md`. After each merge, the
release workflow (`.github/workflows/release.yml`) runs `scripts/stamp-changelog.mjs`, which turns
that merge's fragments (or, when it added none, its PR title) into a dated section for the version
it mints. One `chore(release): v<x.y.z> [skip ci]` commit adds the sections and deletes the
fragments, the tag goes on the merge commit, and both are pushed together with a token from the
release GitHub App, the `main` ruleset's one bypass actor. A GitHub Release is created from each
section, and a Windows job attaches the zip built from the tagged commit (`Scheme: stamp-on-tag`
in `docs/agents/releases.md`, the scheme the `jwh3times/skills` development flow shares).

## Considered options

- **`per-merge`, where each branch writes the section for the version it predicts with
  `scripts/next-version.mjs`.** The prediction holds only for the branch that merges next. With
  several pull requests open, each claims the same version, so every merge sends the others back
  to renumber and re-run CI. TirelessInvest ran it and moved to this scheme for that reason.
- **Releases cut by hand.** A hardware-tuning tool asks users to trust each build. A tag on every
  merge, built by CI from that exact commit with checksums beside it, says which code a binary
  came from, without a maintainer step that can be skipped.
- **Stamp from the release workflow with `GITHUB_TOKEN`.** The ruleset requires pull requests and
  status checks on `main`, and `GITHUB_TOKEN` cannot bypass a ruleset, so the stamp commit's push
  would be refused. The App adds a variable, a secret and a bypass actor, and is the only bypass.

## Consequences

- Pull requests merge in any order with no renumbering, and Dependabot's merges get a one-bullet
  section from their PR titles.
- `scripts/next-version.mjs` reads a non-JSON version file as one bare version, so the release line
  lives in `VERSION`, not `Cargo.toml`. `scripts/check-cargo-version.mjs` fails CI when
  `[workspace.package].version` differs from it, and the release build sets the workspace version
  to the tag's, so `sol --version` reports the release it came from.
- The scheme needs a three-part tag to walk back to. `v0.0.0` marks the initial commit, which was
  pushed before the scheme existed; the bootstrap merge is the first one stamped, as `v0.1.0`.
- The tag job's stamps take turns in a concurrency group that never cancels a running stamp. A push
  rejected because `main` moved is not a failure: the run behind it stamps both merges and packages
  both releases.
- `main` moves twice per merge: the merge, then the stamp commit about a minute later. The ruleset
  requires branches to be up to date, so updating the next branch waits for the `Release` run.
- Reopen this if releases stop following merges, if anything starts to act on the stamp commit (a
  tag-triggered workflow, a deployer that ignores `[skip ci]`), or if the App's access has to widen
  beyond pushing the stamp commit and tags.
