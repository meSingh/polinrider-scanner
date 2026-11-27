#!/usr/bin/env python3
"""Total downloads and clones, per month and for all time (ADR-0074).

Usage:
    reach.py DATA_DIR OUT_DIR

DATA_DIR is a copy of the traffic branch. Writes OUT_DIR/monthly.csv,
OUT_DIR/README.md and OUT_DIR/summary.md, which the workflow commits under
reach/ on the branch. The numbers are the ones posthog.py sends, built by the
same functions, so the branch and PostHog cannot disagree:

- release downloads: people's downloads of binaries and packages
  (github_downloads, `people`), which already include Homebrew, Scoop,
  winget, the .deb packages and every run of the GitHub action, since each
  of those downloads the release file;
- clones by others, per repository (github_clones_by_others,
  `others_uniques`): GitHub's unique cloners per day, less one on a day
  our own workflows checked the repository out (ADR-0075).

The total adds them. A Homebrew or Scoop install is counted twice in it,
once as a clone of the tap or bucket (on a machine's first install only)
and once as a download; the columns stay apart so either can be left out.

Standard library only.
"""

import collections
import csv
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import posthog  # noqa: E402

COLUMNS = [
    ("release_downloads", None),
    ("clones_scanner", "meSingh/polinrider-scanner"),
    ("clones_homebrew_tap", "meSingh/homebrew-tap"),
    ("clones_scoop_bucket", "meSingh/scoop-bucket"),
    ("clones_action", "meSingh/polinrider-action"),
]
HEADER = ["month"] + [c for c, _ in COLUMNS] + ["total"]
BEFORE = "before the log"
ALL = "all time"

README = """# Reach

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
"""


def table(data_dir):
    events = posthog.clones_by_others(data_dir) + posthog.release_downloads(data_dir)
    months = collections.defaultdict(lambda: collections.Counter())
    for e in events:
        p = e["properties"]
        if e["event"] == "github_downloads":
            months[p["day"][:7]]["release_downloads"] += p["people"]
        elif e["event"] == "github_downloads_before_log" and "platform" in p:
            months[BEFORE]["release_downloads"] += p["people"]
        elif e["event"] == "github_clones_by_others":
            for column, repo in COLUMNS:
                if repo == p["repo"]:
                    months[p["day"][:7]][column] += p["others_uniques"]
    order = ([BEFORE] if BEFORE in months else []) + sorted(m for m in months if m != BEFORE)
    rows, everything = [], collections.Counter()
    for m in order:
        counts = months[m]
        everything.update(counts)
        rows.append(line(m, counts))
    rows.append(line(ALL, everything))
    return rows


def line(month, counts):
    row = {"month": month}
    for column, _ in COLUMNS:
        row[column] = counts.get(column, 0)
    row["total"] = sum(row[c] for c, _ in COLUMNS)
    return row


def summary(rows):
    out = ["## Downloads and clones by month", "",
           "| Month | Release downloads | Scanner clones | Tap clones | Bucket clones | Action clones | Total |",
           "|---|---:|---:|---:|---:|---:|---:|"]
    for r in rows:
        name = f"**{r['month']}**" if r["month"] == ALL else r["month"]
        cells = [r[c] for c, _ in COLUMNS] + [r["total"]]
        if r["month"] == ALL:
            cells = [f"**{c}**" for c in cells]
        out.append("| " + " | ".join([name] + [str(c) for c in cells]) + " |")
    out += ["", "Clones are unique cloners per day, other than us (ADR-0075). "
            "Release downloads already include Homebrew, Scoop, winget and the action.", ""]
    return "\n".join(out)


def main():
    if len(sys.argv) != 3:
        print("usage: reach.py DATA_DIR OUT_DIR", file=sys.stderr)
        sys.exit(2)
    data_dir, out_dir = sys.argv[1:]
    os.makedirs(out_dir, exist_ok=True)
    rows = table(data_dir)
    with open(os.path.join(out_dir, "monthly.csv"), "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=HEADER, quoting=csv.QUOTE_ALL, lineterminator="\n")
        w.writeheader()
        w.writerows(rows)
    with open(os.path.join(out_dir, "README.md"), "w") as f:
        f.write(README)
    with open(os.path.join(out_dir, "summary.md"), "w") as f:
        f.write(summary(rows))


if __name__ == "__main__":
    main()
