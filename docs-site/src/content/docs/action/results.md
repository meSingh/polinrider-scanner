---
title: Reading the PolinRider action's results
description: "What the PolinRider Scanner action shows on a run: the job summary for each verdict, notes on each file, the pull request comment and code scanning alerts."
---

Every run writes a job summary. Each finding is also a note on its file, and
two optional outputs put the result on the pull request and in code
scanning.

## The job summary

Open the run and scroll to the summary. It is one of four.

Clean:

```text
PolinRider Scanner: clean

Nothing found in 1,284 files.
PolinRider 3.2.0, indicators last reviewed 16 November 2026.
```

Infected:

```text
PolinRider Scanner: infected

2 confirmed and 1 to review, in 1,284 files.
PolinRider 3.2.0, indicators last reviewed 16 November 2026.

| Result    | Finding                                                 | File               |
|-----------|---------------------------------------------------------|--------------------|
| CONFIRMED | a build config carries the payload                      | vite.config.js     |
| CONFIRMED | an editor task runs the payload when the folder opens   | .vscode/tasks.json |
| TO REVIEW | mentions something the campaign also uses (windowsHide) | src/app.js         |

What to do next
1. Check every computer that has worked on this repository:
   https://polinriderscanner.com/docs/guides/machine/
2. Remove the payload: run this action with mode: fix, or run
   polinrider clean --apply on a clean computer.
3. Change the secrets this repository's CI can read.
```

In fix mode, step 2 links the pull request it opened, or a line under the
steps says why it opened none ([Fix mode](/docs/action/fix-mode/)).

To review, nothing confirmed:

```text
PolinRider Scanner: to review

Nothing confirmed. 1 to review, in 1,284 files.
PolinRider 3.2.0, indicators last reviewed 16 November 2026.

| Result    | Finding                                                 | File       |
|-----------|---------------------------------------------------------|------------|
| TO REVIEW | mentions something the campaign also uses (windowsHide) | src/app.js |

What to do next
1. Read each file named above and ask whether you put that line there.
2. If one is not yours, treat this repository as infected and check every
   computer that has worked on it:
   https://polinriderscanner.com/docs/guides/machine/
```

Could not run:

```text
PolinRider Scanner: could not run

Nothing was scanned to the end, so this is not a clean result.
<the reason>
```

The "indicators last reviewed" date is the newest weekly indicator review in
the polinrider release the action runs. A newer one reaches you with the next
release and Dependabot's pull request.

## Why a scan could not run

The reason is also an error note on the run:

| Reason | What to do |
| --- | --- |
| ... does not match the SHA-256 this action pins, so it was not run | Do not work around it. Report it on the action's issues page |
| Could not download ... | GitHub's downloads were unreachable. Run it again |
| ... does not hold polinrider and its indicators. It was not run. | Report it |
| polinrider 3.2.0 did not start on this runner ... On Linux it needs glibc 2.39 or later, as on ubuntu-24.04. | Use `ubuntu-24.04` or `ubuntu-latest` |
| The archive ... reports version ... It was not run. | Report it |
| This runner is Windows ... | Use a Linux or macOS runner |
| action.yml has no SHA-256 for ..., so nothing was downloaded. | That runner is not supported yet |
| polinrider stopped with exit code ... and no result. | Run it again. If it repeats, report it with the log |
| mode must be check or fix ..., fail-on must be confirmed, review or never ..., or path ... is not a directory in this checkout | Fix the input ([Inputs](/docs/action/inputs/)) |

Report problems on the
[action's issues page](https://github.com/meSingh/polinrider-action/issues).

## Notes on each file

Each finding is a note on its line in the run and in the pull request's Files
tab: an error for a confirmed finding, a warning for one to review.

```text
CONFIRMED  a build config carries the payload
TO REVIEW  mentions something the campaign also uses (windowsHide)
```

What each finding means is in
[Is my repository infected?](/docs/found/is-my-repository-infected/) and
[False positives you will see](/docs/reference/false-positives/).

## A comment on the pull request

`comment: true` (with `pull-requests: write`) posts the job summary on the
pull request and edits that same comment on every later run, so a pull
request never collects a pile of them. A pull request from a fork gets a
read-only token, so it gets no comment; the job summary still has the result.

## Code scanning

`sarif-file` writes the findings as SARIF, and GitHub's `upload-sarif` action
puts them in the repository's Security tab:

```yaml
    permissions:
      contents: read
      pull-requests: write
      security-events: write
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - uses: meSingh/polinrider-action@8c71848593f0e58416c5f61845b3bd2e8080d438 # v1.0.0
        with:
          comment: true
          sarif-file: polinrider.sarif
      - if: ${{ !cancelled() }}
        uses: github/codeql-action/upload-sarif@<sha> # v4
        with:
          sarif_file: polinrider.sarif
```

Pin `upload-sarif` by its commit SHA too. Two rules show up in code scanning:

- `polinrider/confirmed`, "A confirmed PolinRider indicator". Its help points
  to checking every computer that has worked on the repository.
- `polinrider/review`, "Something the PolinRider campaign also uses, for a
  person to look at". Read the line and ask whether you put it there. If not,
  treat the repository as infected.
