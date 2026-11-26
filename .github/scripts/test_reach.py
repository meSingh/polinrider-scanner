#!/usr/bin/env python3
"""Tests for reach.py, against testdata/traffic (see test_posthog.py).

    python3 -m unittest discover -s .github/scripts -p 'test_*.py'
"""

import csv
import os
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import posthog  # noqa: E402
import reach  # noqa: E402

FIXTURE = os.environ.get("TRAFFIC_DATA") or os.path.join(HERE, "testdata", "traffic")


class Table(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.rows = reach.table(FIXTURE)
        cls.events = posthog.build(FIXTURE)

    def test_all_time_is_the_sum_of_the_months(self):
        *months, everything = self.rows
        self.assertEqual(everything["month"], reach.ALL)
        for column in reach.HEADER[1:]:
            self.assertEqual(everything[column], sum(r[column] for r in months), column)

    def test_total_adds_the_columns(self):
        for r in self.rows:
            self.assertEqual(r["total"], sum(r[c] for c, _ in reach.COLUMNS))

    def test_downloads_are_people_s_binaries_and_packages(self):
        people = sum(e["properties"]["people"] for e in self.events
                     if e["event"] == "github_downloads"
                     or (e["event"] == "github_downloads_before_log" and "platform" in e["properties"]))
        self.assertEqual(self.rows[-1]["release_downloads"], people)

    def test_clones_are_others_and_match_posthog(self):
        others = sum(e["properties"]["others"] for e in self.events if e["event"] == "github_clones_by_others")
        clones = sum(self.rows[-1][c] for c, repo in reach.COLUMNS if repo)
        self.assertEqual(clones, others)

    def test_months_in_order_before_the_log_first(self):
        names = [r["month"] for r in self.rows]
        self.assertEqual(names[0], reach.BEFORE)
        self.assertEqual(names[1:-1], sorted(names[1:-1]))

    def test_files_are_written(self):
        with tempfile.TemporaryDirectory() as out:
            sys.argv = ["reach.py", FIXTURE, out]
            reach.main()
            with open(os.path.join(out, "monthly.csv"), newline="") as f:
                rows = list(csv.DictReader(f))
            self.assertEqual(rows[-1]["month"], reach.ALL)
            self.assertEqual(int(rows[-1]["total"]), self.rows[-1]["total"])
            self.assertTrue(os.path.getsize(os.path.join(out, "summary.md")))


if __name__ == "__main__":
    unittest.main()
