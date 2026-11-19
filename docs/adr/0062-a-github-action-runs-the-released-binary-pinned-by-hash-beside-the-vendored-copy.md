# 0062. Offer a GitHub action that runs the released binary pinned by hash, beside the vendored copy

| | |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-11-20 |

## Context

`ci/install-workflow.sh` copies a shell port of the scanner into a
repository, and `ci/README.md` argues against a third-party action: a scan
step that fetches someone else's code on every push is the same shape as the
attack it is meant to catch. That argument still holds, and the copy stays.

But the copy has costs the argument does not count. It is a shell port, not
the engine people run on a laptop, so its verdict can differ from the
binary's (`ci/README.md` lists where). It falls behind until somebody re-runs
the installer in each repository. And it is ten files in somebody else's
repository, which is a lot to ask of a team that only wants a red cross on an
infected push.

Meanwhile an action already exists on the Marketplace under the campaign's
name (Innovative-VAS, "PolinRider Malware Scanner"). It keeps its own
hand-written signature list, its fix mode can amend and force-push the
scanned branch, and its documentation pins the moving `@v1` tag. People who
search the Marketplace find that one.

## Decision

Offer both, and say what each one trusts.

- **The action lives in its own repository, `meSingh/polinrider-action`.** The
  Marketplace lists one action per repository, from an `action.yml` at its
  root, and users then fetch a few small files instead of this repository.
- **It runs the released binary and nothing else.** A composite action in
  bash, with no npm and no other action inside it. It downloads the
  `polinrider` archive for the runner from this repository's release. The
  version, the date of the newest indicator review in that release and the
  SHA-256 of each archive are written into `action.yml`; the archive is
  hashed before it is unpacked, a mismatch stops the run and the binary must
  start and report the pinned version before anything is scanned.
- **Same engine, same verdict.** It runs `polinrider check --fs-only` on the
  checkout, with an empty home directory so the verdict is about the
  repository and not the runner, and maps exit codes 0, 1, 2 and 3 as the
  laptop tool does. Any other exit, or a report with no result, is "could not
  run", which always fails the step: nothing scanned is never clean.
- **Check mode is read-only.** It needs `contents: read`. The scan step gets
  no token; only the steps that open a pull request or post a comment do, and
  they never run the binary.
- **Fix mode opens a pull request and does nothing else.** It runs `clean
  --apply` on the runner's checkout, creates `polinrider/remove-payload-<sha>`
  at the scanned commit, makes one commit on it through GitHub's GraphQL
  `createCommitOnBranch` (signed by GitHub, so no git credentials on the
  runner) and opens a pull request into the scanned branch. It never commits
  to the scanned branch, never force-pushes, never merges, does nothing on a
  pull request from a fork and never uploads the quarantine, which holds the
  payload and stays in the runner's temporary directory.
- **The release workflow keeps it current.** After each final release the
  `packages` job opens a pull request on the action's repository, as it does
  for the tap and the bucket: `packaging/action.sh` rewrites the pinned lines
  from the release's `SHA256SUMS`, and `bump.sh` commits through GitHub's API.
  The maintainer merges it and tags a release of the action. Pre-release tags
  are skipped.
- **Users pin the action by commit SHA**, with Dependabot moving the pin, and
  make the check required on their default branch. Tags of the action never
  move, as with the scanner's.
- **The vendored copy stays as the trust-nothing option**, and the CI guide
  and `ci/README.md` show both with what each one trusts.

## Consequences

People get the laptop engine's verdict in CI with two lines of YAML, and the
indicators reach every repository using the action as soon as its bump is
merged and Dependabot moves their pin, without anyone re-running an
installer. The fix is something a person reviews, never a rewrite of their
branch.

What it costs:

- **It is the shape `ci/README.md` warns about.** A user trusts
  `meSingh/polinrider-action` at the SHA they pin, this repository's release
  on GitHub and the hashes in `action.yml`. A compromise of the maintainer's
  account could publish a new action release with new hashes. Pinning by SHA
  and reviewing Dependabot's pull requests is the defence, and the vendored
  copy remains for anyone who will not take that trade.
- **A download at every run.** If GitHub's release downloads are down, the
  scan cannot run and the step fails rather than passing. Each run is also
  counted as a download in the release history, which is real use.
- **The Linux binaries need glibc 2.39**, so `ubuntu-22.04` runners cannot
  start them. The step says so instead of passing. Windows runners are not
  supported in the first version.
- **Fix mode's pull request waits for approval.** One opened with the default
  token starts its workflow runs only after someone approves them, and the
  repository must allow GitHub Actions to create pull requests. A payload in
  `.github/workflows/` needs a token with the `workflows` permission.
- **Fix mode scans what is checked out.** On `pull_request` the default
  checkout is the merge commit, which no branch points at, so fix mode needs
  the head branch checked out. History and other branches are not scanned;
  the GitHub check run from a laptop does that.
- **The action parses the binary's report.** It reads the findings from the
  `CONFIRMED` and `TO REVIEW` lines at the top of the report (ADR-0059) and
  the counts from `== RESULT ==`. A change to those lines is a change to the
  action, and its fixture tests will say so.
- **One more repository for the packaging token.** `PACKAGING_TOKEN` gains
  write access to the action's repository. Until it has it, the step prints a
  notice and passes.
- **A second release to cut.** Each scanner release now ends with a merge and
  a tag in the action's repository, and the first one with ticking "Publish
  this Action to the GitHub Marketplace", which only the maintainer can do.

## Related

- ADR-0043, which bumps the tap and the bucket by pull request; the action is
  bumped the same way.
- ADR-0036, the GitHub check run from a laptop, which reads every branch and
  the history. The action scans one checkout.
- ADR-0059, which sets the report's shape the action reads.
- ADR-0002, exit code 3 means the scan could not run.
