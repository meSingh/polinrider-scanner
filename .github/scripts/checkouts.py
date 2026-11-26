#!/usr/bin/env python3
"""Count the clones our own workflows make, per repository per day.

GitHub's clone count for a repository includes every `actions/checkout` our
own workflows run, which on a busy day is most of it. To tell other people's
clones from ours (ADR-0074), this counts, for each UTC day, the checkout
steps that ran in the workflow runs of this repository and of the action,
and writes them to clones-ours.csv on the traffic branch. posthog.py and
reach.py take them off GitHub's clone counts.

Usage:
    checkouts.py OLD_CSV NEW_CSV --first-day YYYY-MM-DD --today YYYY-MM-DD

OLD_CSV is clones-ours.csv as it is on the branch (it may not exist yet).
Each day from --first-day to two days before --today that it lacks is
counted and added, oldest first; a day is never counted again. Two days
back, so that every run started that day has finished. A day takes one
request per workflow run, so the first run, which counts the whole log,
stops when the token has fewer than RESERVE requests left and the next run
carries on from there.

How a checkout is counted:
- a step whose name contains "checkout" or "check out", that succeeded,
  in any job of any run created that day, every attempt included, on the
  day the step started; the "Post" cleanup of a checkout is not one;
- a checkout in a job of SUBMODULE_JOBS also fetches the action, which is
  this repository's action/ submodule, so it counts once for each.

Environment: GH_TOKEN, a token that can read the runs of both repositories
(TRAFFIC_TOKEN in the workflow; any token reads a public repository's
runs). Standard library only.
"""

import argparse
import csv
import datetime as dt
import json
import os
import sys
import urllib.error
import urllib.request

API = "https://api.github.com"
SCANNER = "meSingh/polinrider-scanner"
ACTION = "meSingh/polinrider-action"
# The repositories whose own workflows clone them. The tap and the bucket
# run no workflow, so every clone of theirs is someone else's.
REPOS = (SCANNER, ACTION)
# (repository, workflow, job) whose checkout also fetches the action as a
# submodule: ci.yml's "Test the action" (submodules: true).
SUBMODULE_JOBS = {(SCANNER, "CI", "Test the action"): ACTION}
HEADER = ["date", "repository", "checkouts"]
RESERVE = 200


def fail(message):
    print(f"checkouts.py: {message}", file=sys.stderr)
    sys.exit(1)


def is_checkout(step):
    name = step.get("name", "").lower()
    if name.startswith("post "):
        return False
    return ("checkout" in name or "check out" in name) and step.get("conclusion") == "success"


def count(repo, runs, day):
    """Checkouts that started on `day`, per repository they clone.

    `runs` is a list of (workflow name, jobs) for the runs created that day,
    each jobs list as the API gives it, every attempt included."""
    out = {r: 0 for r in REPOS}
    for workflow, jobs in runs:
        for job in jobs:
            for step in job.get("steps") or []:
                if not is_checkout(step):
                    continue
                started = (step.get("started_at") or job.get("started_at") or "")[:10]
                if started != day.isoformat():
                    continue
                out[repo] += 1
                also = SUBMODULE_JOBS.get((repo, workflow, job.get("name")))
                if also:
                    out[also] += 1
    return out


class Client:
    def __init__(self, token):
        self.token = token
        self.remaining = None

    def get(self, path):
        request = urllib.request.Request(f"{API}{path}", headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {self.token}",
            "X-GitHub-Api-Version": "2022-11-28",
            "User-Agent": "polinrider-traffic-log",
        })
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                left = response.headers.get("X-RateLimit-Remaining")
                if left is not None:
                    self.remaining = int(left)
                return json.load(response)
        except urllib.error.HTTPError as e:
            fail(f"GET {path}: HTTP {e.code}")

    def pages(self, path, field):
        page, items = 1, []
        while True:
            sep = "&" if "?" in path else "?"
            body = self.get(f"{path}{sep}per_page=100&page={page}")
            batch = body[field]
            items += batch
            if len(batch) < 100:
                return items
            page += 1

    def runs_of(self, repo, day):
        runs = self.pages(f"/repos/{repo}/actions/runs?created={day.isoformat()}", "workflow_runs")
        return [(run["name"], self.pages(f"/repos/{repo}/actions/runs/{run['id']}/jobs?filter=all", "jobs"))
                for run in runs]


def read(path):
    if not os.path.exists(path):
        return []
    with open(path, newline="") as f:
        reader = csv.DictReader(f)
        if reader.fieldnames != HEADER:
            fail(f"{path} has the header {reader.fieldnames}, expected {HEADER}")
        return list(reader)


def write(path, rows):
    rows = sorted(rows, key=lambda r: (r["date"], r["repository"]))
    with open(path, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=HEADER, quoting=csv.QUOTE_ALL, lineterminator="\n")
        w.writeheader()
        w.writerows(rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("old")
    parser.add_argument("new")
    parser.add_argument("--first-day", required=True, type=dt.date.fromisoformat)
    parser.add_argument("--today", required=True, type=dt.date.fromisoformat)
    args = parser.parse_args()
    token = os.environ.get("GH_TOKEN", "")
    if not token:
        fail("GH_TOKEN is not set")

    rows = read(args.old)
    done = {r["date"] for r in rows}
    last = args.today - dt.timedelta(days=2)
    todo = [args.first_day + dt.timedelta(days=i) for i in range((last - args.first_day).days + 1)]
    todo = [d for d in todo if d.isoformat() not in done]

    client = Client(token)
    added = 0
    for day in todo:
        if client.remaining is not None and client.remaining < RESERVE:
            print(f"Stopped with {client.remaining} requests left; {len(todo) - added} day(s) to count on the next run.")
            break
        totals = {r: 0 for r in REPOS}
        for repo in REPOS:
            for target, n in count(repo, client.runs_of(repo, day), day).items():
                totals[target] += n
        rows += [{"date": day.isoformat(), "repository": r, "checkouts": str(totals[r])} for r in REPOS]
        added += 1
    write(args.new, rows)
    print(f"Counted our own checkouts for {added} day(s).")


if __name__ == "__main__":
    main()
