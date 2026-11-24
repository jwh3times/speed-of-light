# Releases

## Scheme

stamp-on-tag — `.github/workflows/release.yml` tags every merge to `main`
`v<major>.<minor>.<build>` with a GitHub Release, and its `package` job attaches the Windows x64
zip built from the tagged merge commit, with `SHA256SUMS.txt`. Each branch's entries live in its
own `changelog.d/<branch-slug>.md` fragment; the workflow stamps them into the merge's dated section
and tags the merge commit (`changelog.d/README.md`;
[ADR 0001](../adr/0001-stamp-changelog-on-tag.md)). `Unreleased` stays an empty placeholder.
`v0.0.0` marks the initial commit, which predates the scheme and has no section.

## Version file

`VERSION` — the bare `<major>.<minor>.<build>` release line. `Cargo.toml`'s
`[workspace.package].version` names the same line (`scripts/check-cargo-version.mjs`, run by CI's
`agents` job); the release build sets it to the tag's full version, so the binaries report it. It
starts at `0.1.0`.

## Bump command

`node scripts/check-cargo-version.mjs --set <version> && cargo update --workspace` — writes
`VERSION` and the Cargo workspace version, then refreshes the workspace crates' entries in
`Cargo.lock` (no other dependency moves). Running it twice with the same version changes nothing
the second time.

## Ask before bumping

when ambiguous
