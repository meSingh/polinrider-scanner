# Reach

Written daily by .github/workflows/traffic.yml on main, by
.github/scripts/reach.py. Data only.

`monthly.csv`, one row per calendar month (UTC), then one for all time:

- `release_downloads`: people's downloads of the release binaries and
  packages, our own taken off (downloads/). These include Homebrew, Scoop,
  winget, the .deb packages and every run of the GitHub action, which all
  download the release file. Checksums and attestations are left out.
- `clones_*`: unique cloners of each repository other than us, per day,
  summed: GitHub's unique cloners, less one on a day our own workflows
  checked the repository out (clones-ours.csv). Our checkouts collapse into
  very few uniques, so they are taken as one cloner. The tap and the bucket
  run no workflow, so all their cloners are others. Someone who clones on
  three days of a month counts three times: GitHub gives uniques per day
  only.
- `total`: the sum. A Homebrew or Scoop install is in it twice, as a clone
  of the tap or bucket on a machine's first install and as a download.

The first row, "before the log", is the release downloads counted before
the download log began, which have no month. Clones are logged from the
first day of clones.json; GitHub keeps no clone counts older than 14 days,
so none exist before it. A day of the scanner's or the action's clones is
added only once its checkouts are counted, two days after it.
