# Merging

From an open pull request to a merged, released and packaged version. What CI runs is in
`open-pr.md`; this file holds the steps around a merge and what has gone wrong.

## Before merging

- **Bound every wait on checks.** A check can stay `pending` in `gh pr checks` (and `IN_PROGRESS` in
  `statusCheckRollup`) after its job finished, so `gh pr checks --watch` hangs. When the run reads
  `completed` and one check is still pending, read the job directly
  (`gh api repos/jwh3times/speed-of-light/actions/jobs/<id>`): merge when it has a `conclusion` of
  success and the PR's `mergeStateStatus` is `CLEAN`.
- **`BLOCKED` with every check green is usually a new CodeQL alert.** The `main` ruleset blocks a
  merge on any new alert (errors and warnings, or medium security severity and up), and neither
  `gh pr view` nor GraphQL says why. Query the PR's head ref, not its merge ref:
  `gh api "repos/jwh3times/speed-of-light/code-scanning/alerts?ref=refs/pull/<N>/head&state=open"`,
  and read the `CodeQL` check run's summary ("1 new alert…"). A job with no `permissions:` of its
  own, in a workflow with no top-level block, raises `actions/missing-workflow-permissions`; both
  workflows set one, so a new workflow needs it too.
- **The `rust` job runs on `windows-latest`** and takes longer than the ubuntu jobs; a cold cache
  (a `Cargo.lock` change) adds a few minutes. It is the only job that builds the workspace.

## Merging several PRs

`main` requires branches up to date, and auto-merge is disabled for the repository (`--auto` is
refused). Once one PR merges, the next shows `BEHIND`: `gh pr update-branch <n>`, poll
`mergeStateStatus` until `CLEAN`, then `gh pr merge <n> --merge`.

- **PRs merge in any order.** No branch names a version or edits `CHANGELOG.md`: each writes its
  own `changelog.d/<branch-slug>.md` fragment
  ([ADR 0001](../adr/0001-stamp-changelog-on-tag.md)), so open PRs don't collide over the changelog.
- **Wait for the stamp before updating the next branch.** About a minute after each merge, the
  release workflow's `tag` job pushes a `chore(release): v<x.y.z> [skip ci]` commit that moves
  `main` again. Run `gh pr update-branch` after that, or the branch needs a second update and a
  second CI run.

## After the merge

**Find the release run by commit.** Select the `Release` run whose `headSha` is the merge commit
(`gh run list --workflow release.yml --json databaseId,headSha,status,conclusion`); right after a
merge, `--limit 1` can return the previous one. Its `tag` job stamps the merge's section into
`CHANGELOG.md`, deletes its fragment, pushes the stamp commit as the release App's bot, tags the
merge commit and creates the GitHub Release. Confirm with `git tag --points-at <merge commit>`: the
tag is on the merge, not on `main`'s new tip. A run whose stamp found `main` moved ends green with a
notice and skips `package`; the next merge's run releases and packages both.

**Then the `package` job** builds each released tag on Windows from the tag itself and uploads
`speed-of-light-v<x.y.z>-windows-x64.zip` and `SHA256SUMS.txt` to its Release. Confirm with
`gh release view v<x.y.z> --json assets --jq '.assets[].name'`. Its "The binaries report the tag"
step fails when `sol --version` or `sol-service --version` does not print the tag's version, which
means the version stamping (`scripts/check-cargo-version.mjs --set`) did not reach the build. A
failed `package` job leaves the Release without its zip: re-run that job; `--clobber` replaces any
partial upload.

**The stamp commit runs no workflow.** Its `[skip ci]` keeps CI and `Release` off it, so its CodeQL
analysis is the only run it shows. Nothing deploys from `main`.

## `CHANGELOG.md`

No branch writes its sections: the release workflow stamps each merge's section, and the changelog
check fails a branch that adds, removes or renumbers a section or adds entries to `Unreleased`.

## Release workflow

`release.yml`
