# 0074. Total reach counts release downloads and clones by others

| | |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-11-26 |

## Context

PostHog showed one usage number, release downloads (ADR-0053, ADR-0066).
The maintainer asked for a total that also counts clones, Homebrew, Scoop
and the action.

Homebrew, Scoop, winget, the `.deb` packages and the GitHub action all
download the release file from its `releases/download/` link, so GitHub
already counts every one of those installs as a release download. Neither
Homebrew (for a third-party tap) nor Scoop publishes install counts, and the
binary sends nothing home, so installs cannot be counted any other way.

What the download number leaves out is clones: of this repository, by
people who build from source or read the code, and of the tap, the bucket
and the action. GitHub's clone count, though, includes every
`actions/checkout` our own workflows run. On 2026-11-21 that was 209 of the
scanner's 362 clones and 50 of the action's 132, so raw clones would mostly
count us.

## Decision

- `checkouts.py` counts, per UTC day, the successful checkout steps of the
  workflow runs of this repository and of the action, and writes them to
  `clones-ours.csv` on the traffic branch. The "Test the action" job's
  checkout fetches the action as a submodule and counts once for each. A
  day is counted two days after it, once its runs have finished, and never
  again.
- `posthog.py` sends `github_clones_by_others` per repository per day:
  GitHub's clones less ours, never below zero. A day of the scanner or the
  action is sent only once its checkouts are counted. The tap and the
  bucket run no workflow, so all their clones are others'.
- `reach.py` writes `reach/monthly.csv` on the branch: per calendar month
  and for all time, people's release downloads of binaries and packages,
  clones by others per repository, and their total. It reads the same
  functions as `posthog.py`, so the branch and PostHog agree.
- In PostHog the total is a monthly insight and an all-time number, built
  from `github_downloads` (`people`), `github_downloads_before_log`
  (`people`, binaries and packages) and `github_clones_by_others`
  (`others`).

## Consequences

- The total counts a Homebrew or Scoop install twice: as a clone of the tap
  or bucket on a machine's first install, and as a download. The maintainer
  chose to include those clones; the columns stay apart in
  `reach/monthly.csv` and in PostHog so the overlap can be read off.
- Clones by others still include bots, mirrors and services that clone
  public repositories, and our own clones made outside a workflow, such as
  a `git fetch` in the `action/` submodule. It is an upper bound on people.
- A checkout whose step is renamed so that it no longer says "checkout" or
  "check out" is no longer counted as ours. A new job that checks out with
  submodules must be added to `SUBMODULE_JOBS`.
- Counting the whole log the first time takes about 1,300 requests, with
  `TRAFFIC_TOKEN`. The script stops with 200 requests left and the next run
  carries on.
- No clone counts exist before the traffic log began (2026-09-28): GitHub
  keeps 14 days.
