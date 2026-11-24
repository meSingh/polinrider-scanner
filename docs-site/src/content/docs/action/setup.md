---
title: Set up the PolinRider GitHub action
description: "Add the PolinRider Scanner action to a repository: the workflow, pinning it by commit SHA, Dependabot for the pin, a required check and the weekly scan."
---

The [PolinRider Scanner action](/github-action/) scans a repository on every
push, every pull request and once a week, with the same polinrider binary and
indicators as the laptop tool. This page sets it up so that a reinfection
cannot merge. Five minutes, in one repository at a time.

## Add the workflow

Add `.github/workflows/polinrider.yml`:

```yaml
name: PolinRider
on:
  push:
  pull_request:
  schedule:
    - cron: "17 3 * * 1"
permissions:
  contents: read
jobs:
  scan:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - uses: meSingh/polinrider-action@de52596b9ced60ae70152745d03fa5c36464d329 # v1.1.1
```

What each part does:

- `push` and `pull_request` scan every change before and after it merges.
- `schedule` scans on Mondays at 03:17 UTC even when nobody pushes.
- `permissions: contents: read` is all check mode needs.
- `persist-credentials: false` keeps git credentials off the runner. The
  action never needs them, fix mode included.

A confirmed finding fails the `scan` job. What the run shows is in
[Reading the results](/docs/action/results/).

The action checks the commit it runs on. It does not read the repository's
other branches or older commits: [checking a computer](/docs/guides/machine/)
does that, where someone can act on what it finds.

## Pin it by commit SHA

Use the full commit SHA of a release, with its version as a comment. The
action's tags are protected and never move, but a SHA is the one reference
GitHub guarantees cannot change. The
[releases page](https://github.com/meSingh/polinrider-action/releases) lists
each release. To read a release's SHA from the command line:

```bash
gh api repos/meSingh/polinrider-action/commits/v1.1.1 --jq .sha
```

The release notes of each version name the polinrider version it runs.
v1.1.1 runs polinrider 3.3.2.

## Let Dependabot move the pin

Each polinrider release becomes a new release of the action, with the new
version and hashes. Add `.github/dependabot.yml`:

```yaml
version: 2
updates:
  - package-ecosystem: github-actions
    directory: /
    schedule:
      interval: weekly
```

Dependabot then opens a pull request that moves the SHA and the comment.
Review it like any other change: it is the moment new indicators reach your
repository, and the moment to look at what changed in the action.

## Make it a required check

Without this the check is only a warning anyone can merge past.

1. Open the repository's **Settings, Rules, Rulesets** and add a branch
   ruleset.
2. Target the default branch.
3. Turn on **Require status checks to pass** and add `scan`.

The check is named after the job. If you rename the job, pick that name. An
infected push or pull request now cannot merge into the default branch.
[Block a merge on a finding](/docs/action/require/) has the full ruleset, the
command-line version and the organization-wide one.

## Keep the weekly schedule

New indicators arrive with new releases. The Monday run scans code that has
not changed since, so an old file is checked against what is known today.
GitHub turns off scheduled workflows in a public repository after 60 days
without activity; if your repository is quiet, look at the Actions tab now
and then.

## Runners

| Runner | |
| --- | --- |
| `ubuntu-latest`, `ubuntu-24.04` and their arm64 runners | Supported. The Linux binaries need glibc 2.39 or later, so `ubuntu-22.04` cannot start them and the step says so |
| `macos-latest` and the other macOS runners | Supported, with less mileage than Linux so far |
| Windows | Not yet. The step fails and says so |

A self-hosted runner needs bash, curl, tar and a SHA-256 tool. Fix mode and
comments also need `gh` and `jq`, which GitHub's own runners have.

## Next

- [Inputs, outputs and permissions](/docs/action/inputs/) to change what
  fails the check
- [Fix mode](/docs/action/fix-mode/) to have the action open a pull request
  that removes the payload
