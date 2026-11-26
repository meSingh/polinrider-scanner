#!/usr/bin/env python3
"""Tests for posthog.py, against a copy of the traffic branch's data.

    python3 -m unittest discover -s .github/scripts -p 'test_*.py'

testdata/traffic/ is a slice of the traffic branch as it was on 2026-11-21:
the views and clones from 1 November, the last five days of stars, two days
of referrers and paths and the whole downloads/ ledger, plus made-up rows
for the tap (repos/), the action's users and our own checkouts
(clones-ours.csv). To run the same checks against
the whole branch as it is now:

    git worktree add /tmp/traffic origin/traffic
    TRAFFIC_DATA=/tmp/traffic python3 -m unittest discover -s .github/scripts -p 'test_*.py'

Nothing here reaches PostHog: sending is tested against a stand-in server on
127.0.0.1.
"""

import collections
import csv
import datetime as dt
import gzip
import http.server
import io
import json
import os
import shutil
import sys
import tempfile
import threading
import unittest
import uuid
from contextlib import redirect_stderr, redirect_stdout

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import posthog  # noqa: E402

FIXTURE = os.path.join(HERE, "testdata", "traffic")
KEY = "phc_" + "T" * 43  # well-formed, not a key


def week_of(day):
    d = dt.date.fromisoformat(day)
    return (d - dt.timedelta(days=d.weekday())).isoformat()


class Stand(http.server.BaseHTTPRequestHandler):
    """PostHog's /batch/, as far as the sender sees it."""

    def do_POST(self):
        body = self.rfile.read(int(self.headers["Content-Length"]))
        if self.headers.get("Content-Encoding") == "gzip":
            body = gzip.decompress(body)
        payload = json.loads(body)
        self.server.requests.append((self.path, payload))
        status = self.server.answers.pop(0) if self.server.answers else 200
        self.send_response(status)
        self.end_headers()
        self.wfile.write(b'{"status": 1}')

    def log_message(self, *args):
        pass


class Events(unittest.TestCase):
    data = FIXTURE

    @classmethod
    def setUpClass(cls):
        cls.events = posthog.build(cls.data)
        cls.by_name = collections.defaultdict(list)
        for e in cls.events:
            cls.by_name[e["event"]].append(e)

    def test_every_event_is_personless_and_on_a_day(self):
        for e in self.events:
            self.assertEqual(e["distinct_id"], "polinrider-traffic-log")
            self.assertIs(e["properties"]["$process_person_profile"], False)
            self.assertEqual(e["properties"]["source"], "traffic-log")
            day = e["properties"]["day"]
            self.assertEqual(e["timestamp"], f"{day}T00:00:00Z")
            self.assertEqual(uuid.UUID(e["uuid"]).version, 5)

    def test_uuids_are_unique_and_stable(self):
        uuids = [e["uuid"] for e in self.events]
        self.assertEqual(len(uuids), len(set(uuids)))
        self.assertEqual(uuids, [e["uuid"] for e in posthog.build(self.data)])

    def test_views_and_clones_match_the_branch(self):
        for kind in ("views", "clones"):
            with open(os.path.join(self.data, f"{kind}.json")) as f:
                rows = json.load(f)
            ours = {e["properties"]["day"]: e["properties"] for e in self.by_name[f"github_{kind}"]
                    if e["properties"]["repo"] == "meSingh/polinrider-scanner"}
            self.assertEqual(len(ours), len(rows))
            for r in rows:
                p = ours[r["timestamp"][:10]]
                self.assertEqual((p["count"], p["uniques"]), (r["count"], r["uniques"]))

    def test_stars_one_a_day(self):
        with open(os.path.join(self.data, "stars.csv"), newline="") as f:
            rows = list(csv.DictReader(f))
        got = {e["properties"]["day"]: e["properties"]["stars"] for e in self.by_name["github_stars"]}
        self.assertEqual(got, {r["date"]: int(r["stars"]) for r in rows})

    def test_referrers_and_paths_keep_rank(self):
        for folder, name, field in (("referrers", "github_top_referrer", "referrer"),
                                    ("paths", "github_top_path", "path")):
            for file in os.listdir(os.path.join(self.data, folder)):
                day = file[:10]
                with open(os.path.join(self.data, folder, file)) as f:
                    rows = json.load(f)
                got = sorted((e["properties"]["rank"], e["properties"][field])
                             for e in self.by_name[name] if e["properties"]["day"] == day)
                self.assertEqual(got, [(i, r[field]) for i, r in enumerate(rows, start=1)])
                for e in self.by_name[name]:
                    self.assertEqual(e["properties"]["period_days"], 14)

    def test_downloads_add_up_to_the_weekly_table(self):
        """Per day, per release and platform, the events sum to weekly.csv,
        which downloads.py wrote from the same ledger."""
        weekly = collections.defaultdict(lambda: [0, 0, 0, 0])
        for e in self.by_name["github_downloads"]:
            p = e["properties"]
            weekly[week_of(p["day"])][0] += p["downloads"]
            weekly[week_of(p["day"])][1] += p["ours"]
        for e in self.by_name["github_checksum_downloads"]:
            p = e["properties"]
            weekly[week_of(p["day"])][2] += p["downloads"]
            weekly[week_of(p["day"])][3] += p["ours"]
        first = None
        for e in self.by_name["github_downloads_before_log"]:
            p = e["properties"]
            first = p["day"]
            i = 0 if p["kind"] == "binaries and packages" else 2
            weekly[f"before {first}"][i] += p["downloads"]
            weekly[f"before {first}"][i + 1] += p["ours"]
        with open(os.path.join(self.data, "downloads", "weekly.csv"), newline="") as f:
            table = list(csv.DictReader(f))
        self.assertTrue(table)
        for row in table:
            d, o, v, vo = weekly.pop(row["week"])
            self.assertEqual((d, o, d - o, v - vo),
                             (int(row["downloads"]), int(row["ours"]), int(row["people"]),
                              int(row["checksums_and_attestations"])), row["week"])
        # The branch's weekly.csv is written by the same run that took the
        # latest totals; any week left over must be empty.
        self.assertFalse({k: v for k, v in weekly.items() if any(v)})

    def test_checksums_are_apart(self):
        for e in self.by_name["github_downloads"]:
            self.assertNotEqual(e["properties"]["platform"], "checksums and attestations")
        for e in self.by_name["github_checksum_downloads"]:
            f = e["properties"]["file"]
            self.assertTrue(f == "SHA256SUMS" or f.endswith(".intoto.jsonl"), f)


class FixtureOnly(unittest.TestCase):
    """Checks that depend on the made-up rows in testdata/."""

    def test_other_repositories_and_action_users(self):
        events = posthog.build(FIXTURE)
        tap = [e for e in events if e["properties"].get("repo") == "meSingh/homebrew-tap"
               and e["event"] in ("github_views", "github_clones")]
        self.assertEqual(sorted((e["event"], e["properties"]["day"], e["properties"]["count"]) for e in tap),
                         [("github_clones", "2026-11-19", 7), ("github_views", "2026-11-19", 4),
                          ("github_views", "2026-11-20", 0)])
        users = {e["properties"]["day"]: (e["properties"]["repositories"], e["properties"]["files"])
                 for e in events if e["event"] == "github_action_users"}
        self.assertEqual(users, {"2026-11-20": (0, 0), "2026-11-21": (2, 3)})

    def test_clones_by_others_take_our_checkouts_off(self):
        got = sorted((e["properties"]["repo"], e["properties"]["day"], e["properties"]["clones"],
                      e["properties"]["ours"], e["properties"]["others"])
                     for e in posthog.build(FIXTURE) if e["event"] == "github_clones_by_others")
        # The scanner only on the days clones-ours.csv has, never below zero
        # (40 of ours against 34 clones on the 18th); the tap, which runs no
        # workflow, on every day.
        self.assertEqual(got, [("meSingh/homebrew-tap", "2026-11-19", 7, 0, 7),
                               ("meSingh/polinrider-scanner", "2026-11-17", 66, 22, 44),
                               ("meSingh/polinrider-scanner", "2026-11-18", 34, 40, 0)])

    def test_a_revised_count_keeps_its_uuid(self):
        with tempfile.TemporaryDirectory() as tmp:
            data = os.path.join(tmp, "data")
            shutil.copytree(FIXTURE, data)
            before = {e["uuid"]: e for e in posthog.build(data)}
            path = os.path.join(data, "views.json")
            with open(path) as f:
                rows = json.load(f)
            rows[-1]["count"] += 5
            with open(path, "w") as f:
                json.dump(rows, f)
            after = {e["uuid"]: e for e in posthog.build(data)}
            self.assertEqual(before.keys(), after.keys())
            changed = [u for u in after if posthog.digest(after[u]) != posthog.digest(before[u])]
            self.assertEqual(len(changed), 1)
            self.assertEqual(after[changed[0]]["event"], "github_views")

    def test_our_own_downloads_are_taken_off(self):
        with tempfile.TemporaryDirectory() as tmp:
            data = os.path.join(tmp, "data")
            shutil.copytree(FIXTURE, data)
            with open(os.path.join(data, "downloads", "ours.csv"), "a") as f:
                f.write("2026-11-18,v3.1.0,polinrider-v3.1.0-macos-arm64.tar.gz,2,release check\n")
            hit = [e["properties"] for e in posthog.build(data)
                   if e["event"] == "github_downloads" and e["properties"]["day"] == "2026-11-18"
                   and e["properties"]["tag"] == "v3.1.0" and e["properties"]["platform"] == "macos-arm64.tar.gz"]
            self.assertEqual(len(hit), 1)
            self.assertEqual(hit[0]["ours"], 2)
            self.assertEqual(hit[0]["people"], hit[0]["downloads"] - 2)


class Sending(unittest.TestCase):
    def setUp(self):
        self.server = http.server.HTTPServer(("127.0.0.1", 0), Stand)
        self.server.requests, self.server.answers = [], []
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.tmp = tempfile.mkdtemp()
        self.data = os.path.join(self.tmp, "data")
        shutil.copytree(FIXTURE, self.data)
        self.env = dict(os.environ)
        os.environ["POSTHOG_PROJECT_KEY"] = KEY
        os.environ["POSTHOG_HOST"] = f"http://127.0.0.1:{self.server.server_port}"
        posthog.time.sleep = lambda s: None

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        shutil.rmtree(self.tmp)
        os.environ.clear()
        os.environ.update(self.env)

    def run_send(self, everything=False):
        out = os.path.join(self.tmp, "sent.csv")
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            status = posthog.send(self.data, out, everything)
        return status, out

    def keep(self, ledger):
        os.makedirs(os.path.join(self.data, "posthog"), exist_ok=True)
        shutil.copy(ledger, os.path.join(self.data, "posthog", "sent.csv"))

    def sent(self):
        return [e for _, p in self.server.requests for e in p["batch"]]

    def test_first_send_sends_everything_once(self):
        status, ledger = self.run_send()
        self.assertEqual(status, 0)
        events = posthog.build(self.data)
        got = self.sent()
        self.assertEqual(sorted(e["uuid"] for e in got), sorted(e["uuid"] for e in events))
        for path, payload in self.server.requests:
            self.assertEqual(path, "/batch/")
            self.assertEqual(payload["api_key"], KEY)
            self.assertLessEqual(len(payload["batch"]), posthog.BATCH_SIZE)
            for e in payload["batch"]:
                self.assertEqual(e["properties"]["distinct_id"], "polinrider-traffic-log")
                self.assertEqual(e["type"], "capture")
        # Older than two days goes as a historical migration, the rest not.
        cutoff = (dt.datetime.now(dt.timezone.utc) - dt.timedelta(days=2)).strftime("%Y-%m-%dT%H:%M:%SZ")
        for _, p in self.server.requests:
            for e in p["batch"]:
                self.assertEqual(p["historical_migration"], e["timestamp"] < cutoff, e["timestamp"])
        self.assertTrue(any(p["historical_migration"] for _, p in self.server.requests))
        self.assertEqual(len(posthog.read_ledger(ledger)), len(events))

    def test_nothing_new_sends_nothing_and_a_change_sends_one(self):
        _, ledger = self.run_send()
        self.keep(ledger)
        self.server.requests.clear()
        status, _ = self.run_send()
        self.assertEqual((status, self.sent()), (0, []))
        with open(os.path.join(self.data, "stars.csv"), "a") as f:
            f.write(f'"{dt.date.today().isoformat()}",9,4,1\n')
        status, _ = self.run_send()
        got = self.sent()
        self.assertEqual([e["event"] for e in got], ["github_stars"])
        # Today is recent: no historical flag.
        self.assertFalse(self.server.requests[-1][1]["historical_migration"])

    def test_backfill_sends_everything_again(self):
        _, ledger = self.run_send()
        self.keep(ledger)
        self.server.requests.clear()
        status, _ = self.run_send(everything=True)
        self.assertEqual(status, 0)
        self.assertEqual(len(self.sent()), len(posthog.build(self.data)))

    def test_a_refused_batch_keeps_what_went(self):
        self.server.answers = [200, 400]
        status, ledger = self.run_send()
        self.assertEqual(status, 1)
        self.assertEqual(len(posthog.read_ledger(ledger)), posthog.BATCH_SIZE)
        self.keep(ledger)
        self.server.requests.clear()
        self.run_send()
        self.assertEqual(len(self.sent()), len(posthog.build(self.data)) - posthog.BATCH_SIZE)

    def test_a_server_error_is_retried(self):
        self.server.answers = [502, 200]
        status, _ = self.run_send()
        self.assertEqual(status, 0)
        self.assertEqual(len({e["uuid"] for e in self.sent()}), len(posthog.build(self.data)))

    def test_no_key_sends_nothing_and_writes_nothing(self):
        os.environ["POSTHOG_PROJECT_KEY"] = ""
        status, ledger = self.run_send()
        self.assertEqual((status, self.server.requests, os.path.exists(ledger)), (0, [], False))

    def test_a_value_that_is_not_a_key_stops(self):
        os.environ["POSTHOG_PROJECT_KEY"] = "not-a-key"
        with self.assertRaises(SystemExit) as caught:
            self.run_send()
        self.assertEqual(caught.exception.code, 1)
        self.assertEqual(self.server.requests, [])


if os.environ.get("TRAFFIC_DATA"):
    class WholeBranch(Events):
        """The same checks against a checkout of the whole traffic branch."""
        data = os.environ["TRAFFIC_DATA"]


if __name__ == "__main__":
    unittest.main()
