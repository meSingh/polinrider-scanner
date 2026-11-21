#!/usr/bin/env python3
"""Send the traffic log's numbers to PostHog, one event per day per thing.

The `traffic` branch is the permanent store (ADR-0053, ADR-0066); PostHog is
where the numbers are looked at, beside the website's. Run daily by
.github/workflows/traffic.yml after the branch is written, this reads the
branch as it is and sends what PostHog has not had yet.

Usage:
    posthog.py events   DATA_DIR                  print every event, one JSON per line
    posthog.py send     DATA_DIR OUT_LEDGER       send what is new or changed
    posthog.py backfill DATA_DIR OUT_LEDGER       send the whole history again

DATA_DIR is a copy of the traffic branch. Its posthog/sent.csv, the ledger,
says what was sent: one row per event, its uuid and a digest of its content.
`send` sends the events whose row is missing or whose digest has changed,
and writes the new ledger to OUT_LEDGER. `backfill` sends every event,
whatever the ledger says, for a new PostHog project or one that lost data.

Each event has a uuid made from what it counts (the event name, the day and
what it is about), and a timestamp at midnight UTC of its day. PostHog treats
events with the same uuid, name, timestamp and distinct_id as one, and a
newer one replaces the older (de-duplication happens in the background, so a
duplicate can show for a while before it is merged). So sending an event
again, by a rerun, a backfill or a count that GitHub revised, never counts it
twice. The ledger keeps resends to the events that changed.

Every event has the distinct_id "polinrider-traffic-log" and
$process_person_profile false: no person profile is made.

Environment: POSTHOG_PROJECT_KEY, the project's key (phc_...). With none,
`send` and `backfill` say so and exit 0 without writing a ledger.
POSTHOG_HOST overrides the EU cloud's address, for testing.

Standard library only: the runner's Python, nothing installed.
"""

import argparse
import csv
import datetime as dt
import hashlib
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
import uuid

# downloads.py sits beside this file; the workflow runs Python with -I, which
# leaves the script's own directory off the path.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import downloads  # noqa: E402

HOST = "https://eu.i.posthog.com"
DISTINCT_ID = "polinrider-traffic-log"
REPO = "meSingh/polinrider-scanner"
OWNER = "meSingh"
# Fixed for good: changing it changes every uuid, and PostHog would count
# everything twice.
NAMESPACE = uuid.uuid5(uuid.NAMESPACE_URL, "https://github.com/meSingh/polinrider-scanner/tree/traffic")
LEDGER = os.path.join("posthog", "sent.csv")
LEDGER_HEADER = ["uuid", "digest"]
BATCH_SIZE = 200
KEY_RE = re.compile(r"^phc_[A-Za-z0-9]{20,64}$")
DAY_FILE = re.compile(r"^(\d{4}-\d{2}-\d{2})\.json$")


def fail(message):
    print(f"posthog.py: {message}", file=sys.stderr)
    sys.exit(1)


def read_json(path, fallback):
    if not os.path.exists(path):
        return fallback
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError) as e:
        fail(f"cannot read {path}: {e}")


def event(name, day, key, props):
    """One event. `key` names what it counts within the day; with the name
    and the day it makes the uuid, so the same thing on the same day always
    has the same uuid."""
    ident = "|".join([name, day.isoformat(), *map(str, key)])
    return {
        "event": name,
        "uuid": str(uuid.uuid5(NAMESPACE, ident)),
        "timestamp": f"{day.isoformat()}T00:00:00Z",
        "distinct_id": DISTINCT_ID,
        "properties": {
            "$process_person_profile": False,
            "source": "traffic-log",
            "day": day.isoformat(),
            **props,
        },
    }


def daily_traffic(data_dir):
    """github_views and github_clones: per repository per day, the count and
    the unique visitors or cloners, as GitHub gives them (UTC days)."""
    sources = [(REPO, data_dir)]
    others = os.path.join(data_dir, "repos")
    if os.path.isdir(others):
        sources += [(f"{OWNER}/{name}", os.path.join(others, name))
                    for name in sorted(os.listdir(others))
                    if os.path.isdir(os.path.join(others, name))]
    out = []
    for repo, folder in sources:
        for kind in ("views", "clones"):
            for row in read_json(os.path.join(folder, f"{kind}.json"), []):
                day = downloads.parse_day(str(row.get("timestamp", ""))[:10], f"{repo} {kind} timestamp")
                out.append(event(f"github_{kind}", day, [repo], {
                    "repo": repo,
                    "count": int(row["count"]),
                    "uniques": int(row["uniques"]),
                }))
    return out


def snapshots(data_dir):
    """github_top_referrer and github_top_path: each day's answer, which is
    GitHub's top ten over the 14 days up to that day, not that day's own
    traffic. Summing them across days counts the same visits many times;
    read one day, or the latest."""
    out = []
    for folder, name, field in (("referrers", "github_top_referrer", "referrer"),
                                ("paths", "github_top_path", "path")):
        path = os.path.join(data_dir, folder)
        if not os.path.isdir(path):
            continue
        for file in sorted(os.listdir(path)):
            m = DAY_FILE.match(file)
            if not m:
                continue
            day = downloads.parse_day(m.group(1), f"{folder}/{file}")
            for rank, row in enumerate(read_json(os.path.join(path, file), []), start=1):
                props = {
                    "repo": REPO,
                    field: row[field],
                    "rank": rank,
                    "count": int(row["count"]),
                    "uniques": int(row["uniques"]),
                    "period_days": 14,
                }
                if field == "path":
                    props["title"] = row.get("title", "")
                out.append(event(name, day, [REPO, row[field]], props))
    return out


def stars(data_dir):
    """github_stars: the star, fork and watcher counts as read that day."""
    path = os.path.join(data_dir, "stars.csv")
    if not os.path.exists(path):
        return []
    out = []
    with open(path, newline="") as f:
        for row in csv.DictReader(f):
            day = downloads.parse_day(row["date"], "stars.csv date")
            out.append(event("github_stars", day, [REPO], {
                "repo": REPO,
                "stars": int(row["stars"]),
                "forks": int(row["forks"]),
                "watchers": int(row["watchers"]),
            }))
    return out


def action_users(data_dir):
    """github_action_users: public repositories whose workflows name
    meSingh/polinrider-action, by GitHub code search, the owner's own left
    out. Code search sees only what it has indexed, so this is a floor."""
    path = os.path.join(data_dir, "action-users.csv")
    if not os.path.exists(path):
        return []
    out = []
    with open(path, newline="") as f:
        for row in csv.DictReader(f):
            day = downloads.parse_day(row["date"], "action-users.csv date")
            out.append(event("github_action_users", day, [f"{OWNER}/polinrider-action"], {
                "action": f"{OWNER}/polinrider-action",
                "repositories": int(row["repositories"]),
                "files": int(row["files"]),
            }))
    return out


def release_downloads(data_dir):
    """Release downloads per day, from the same arithmetic as the weekly
    tables (downloads.py): `downloads` as GitHub counted them, `ours` from
    ours.csv, and `people`, the difference, which can be below zero on a day
    when one of ours is recorded on a different day from the one GitHub
    counted it.

    - github_downloads: binaries and packages, per release and platform;
    - github_checksum_downloads: SHA256SUMS and attestation bundles, per
      release and file, kept apart because they are fetched to check a
      download, not to run anything;
    - github_downloads_before_log: everything counted before the log began,
      which has no day, per release and platform or file, on the log's
      first day.
    """
    folder = os.path.join(data_dir, "downloads")
    counts = downloads.read_csv(os.path.join(folder, "counts.csv"), downloads.COUNTS_HEADER)
    ours = downloads.read_csv(os.path.join(folder, "ours.csv"), downloads.OURS_HEADER)
    if not counts:
        return []
    counts.sort(key=lambda r: (r["date"], r["tag"], r["file"]))
    today = max(downloads.parse_day(r["date"], "counts.csv date") for r in counts)
    first_day, changes = downloads.daily_changes(counts, today)

    # (day or None, tag, what) -> [downloads, ours, checksum?]
    totals = {}

    def add(day, tag, name, n, column):
        check = downloads.is_verification(name)
        what = name if check else downloads.platform_of(tag, name)
        cell = totals.setdefault((day, tag, what), [0, 0, check])
        cell[column] += n

    for day, tag, name, n in changes:
        add(day, tag, name, n, 0)
    for r in ours:
        day = downloads.parse_day(r["date"], "ours.csv date")
        add(None if day < first_day else day, r["tag"], r["file"], int(r["count"]), 1)

    out = []
    for (day, tag, what), (n, mine, check) in sorted(
            totals.items(), key=lambda x: (x[0][0] or dt.date.min, x[0][1], x[0][2])):
        props = {"repo": REPO, "tag": tag, "downloads": n, "ours": mine, "people": n - mine}
        props["file" if check else "platform"] = what
        if day is None:
            props["kind"] = "checksums and attestations" if check else "binaries and packages"
            out.append(event("github_downloads_before_log", first_day, [REPO, tag, what], props))
        else:
            name = "github_checksum_downloads" if check else "github_downloads"
            out.append(event(name, day, [REPO, tag, what], props))
    return out


def build(data_dir):
    if not os.path.isdir(data_dir):
        fail(f"{data_dir} is not a folder")
    events = (daily_traffic(data_dir) + stars(data_dir) + snapshots(data_dir)
              + action_users(data_dir) + release_downloads(data_dir))
    seen = {}
    for e in events:
        if e["uuid"] in seen:
            fail(f"two events share the uuid {e['uuid']}: {seen[e['uuid']]} and {e['event']} "
                 f"on {e['properties']['day']}")
        seen[e["uuid"]] = e["event"]
    return events


def digest(e):
    body = json.dumps([e["event"], e["timestamp"], e["properties"]], sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(body.encode()).hexdigest()[:16]


def read_ledger(path):
    return {r["uuid"]: r["digest"] for r in downloads.read_csv(path, LEDGER_HEADER)}


def write_ledger(path, ledger):
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    downloads.write_csv(path, LEDGER_HEADER, sorted(ledger.items()))


def post(host, payload, attempts=4):
    body = json.dumps(payload).encode()
    for attempt in range(1, attempts + 1):
        request = urllib.request.Request(
            f"{host}/batch/", data=body, method="POST",
            headers={"Content-Type": "application/json", "User-Agent": "polinrider-traffic-log"})
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                response.read()
                return
        except urllib.error.HTTPError as e:
            # A 4xx will not change on a retry: a wrong key or a bad event.
            if e.code < 500 or attempt == attempts:
                raise RuntimeError(f"PostHog answered {e.code}: {e.read()[:300]!r}") from None
        except (urllib.error.URLError, TimeoutError) as e:
            if attempt == attempts:
                raise RuntimeError(f"cannot reach PostHog: {e}") from None
        time.sleep(2 ** attempt)


def send(data_dir, out_ledger, everything):
    key = os.environ.get("POSTHOG_PROJECT_KEY", "").strip()
    if not key:
        print("POSTHOG_PROJECT_KEY is not set, so nothing is sent to PostHog.")
        return 0
    if not KEY_RE.match(key):
        fail("POSTHOG_PROJECT_KEY is not a PostHog project key (phc_ followed by letters and digits)")
    host = os.environ.get("POSTHOG_HOST", HOST).rstrip("/")

    events = build(data_dir)
    ledger = read_ledger(os.path.join(data_dir, LEDGER))
    # Events no longer built (none today) keep their rows, so a later change
    # to how one is built cannot make an old event look new.
    todo = [e for e in events if everything or ledger.get(e["uuid"]) != digest(e)]

    # Older than two days goes as a historical migration, which PostHog
    # processes in order and keeps out of its spike detection.
    cutoff = (dt.datetime.now(dt.timezone.utc) - dt.timedelta(days=2)).strftime("%Y-%m-%dT%H:%M:%SZ")
    groups = [(True, [e for e in todo if e["timestamp"] < cutoff]),
              (False, [e for e in todo if e["timestamp"] >= cutoff])]
    sent = 0
    status = 0
    try:
        for historical, group in groups:
            for i in range(0, len(group), BATCH_SIZE):
                batch = group[i:i + BATCH_SIZE]
                post(host, {
                    "api_key": key,
                    "historical_migration": historical,
                    "batch": [{**e, "type": "capture",
                               "properties": {**e["properties"], "distinct_id": DISTINCT_ID}}
                              for e in batch],
                })
                for e in batch:
                    ledger[e["uuid"]] = digest(e)
                sent += len(batch)
    except RuntimeError as e:
        print(f"::error::{e}. Sent {sent} of {len(todo)}; the rest go on the next run.", file=sys.stderr)
        status = 1
    write_ledger(out_ledger, ledger)
    names = {}
    for e in todo[:sent]:
        names[e["event"]] = names.get(e["event"], 0) + 1
    detail = ", ".join(f"{n} {k}" for k, n in sorted(names.items())) or "nothing new"
    print(f"Sent {sent} event(s) to PostHog: {detail}.")
    return status


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="command", required=True)
    p = sub.add_parser("events")
    p.add_argument("data_dir")
    for name in ("send", "backfill"):
        p = sub.add_parser(name)
        p.add_argument("data_dir")
        p.add_argument("out_ledger")
    args = ap.parse_args()

    if args.command == "events":
        for e in build(args.data_dir):
            print(json.dumps(e, sort_keys=True))
        return 0
    return send(args.data_dir, args.out_ledger, everything=args.command == "backfill")


if __name__ == "__main__":
    sys.exit(main())
