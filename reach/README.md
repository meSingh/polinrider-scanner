# Reach

Written daily by .github/workflows/traffic.yml on main, by
.github/scripts/reach.py. Data only.

`monthly.csv`, one row per calendar month (UTC), then one for all time:

- `release_downloads`: people's downloads of the release binaries and
  packages, our own taken off (downloads/). These include Homebrew, Scoop,
  winget, the .deb packages and every run of the GitHub action, which all
  download the release file. Checksums and attestations are left out.
- `clones_*`: clones of each repository by others: GitHub's clone count with
  the checkouts of our own workflows taken off (clones-ours.csv). The tap
  and the bucket run no workflow, so all their clones are others'.
- `total`: the sum. A Homebrew or Scoop install is in it twice, as a clone
  of the tap or bucket on a machine's first install and as a download.

The first row, "before the log", is the release downloads counted before
the download log began, which have no month. Clones are logged from the
first day of clones.json; GitHub keeps no clone counts older than 14 days,
so none exist before it. A day of the scanner's or the action's clones is
added only once its checkouts are counted, two days after it.
