#!/usr/bin/env python3
"""Tests for checkouts.py's counting, on made-up jobs. Nothing reaches GitHub.

    python3 -m unittest discover -s .github/scripts -p 'test_*.py'
"""

import datetime as dt
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import checkouts  # noqa: E402

DAY = dt.date(2026, 11, 21)


def step(name, conclusion="success", started="2026-11-21T10:00:00Z"):
    return {"name": name, "conclusion": conclusion, "started_at": started}


def job(name, *steps):
    return {"name": name, "started_at": "2026-11-21T10:00:00Z", "steps": list(steps)}


class Count(unittest.TestCase):
    def test_a_checkout_counts_once_and_its_cleanup_not(self):
        runs = [("CI", [job("Static analysis",
                            step("Run actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"),
                            step("cargo clippy"),
                            step("Post Run actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1")),
                        job("Release", step("Check out the tag"))])]
        self.assertEqual(checkouts.count(checkouts.SCANNER, runs, DAY),
                         {checkouts.SCANNER: 2, checkouts.ACTION: 0})

    def test_skipped_failed_and_other_days_do_not_count(self):
        runs = [("CI", [job("a", step("Run actions/checkout@x", "skipped")),
                        job("b", step("Run actions/checkout@x", "failure")),
                        job("c", step("Run actions/checkout@x", started="2026-11-22T00:00:01Z")),
                        job("d", {"name": "Check out repository", "conclusion": "success", "started_at": None})])]
        self.assertEqual(checkouts.count(checkouts.SCANNER, runs, DAY),
                         {checkouts.SCANNER: 1, checkouts.ACTION: 0})

    def test_the_submodule_checkout_also_clones_the_action(self):
        runs = [("CI", [job("Test the action", step("Run actions/checkout@x"))]),
                ("Docs", [job("Test the action", step("Run actions/checkout@x"))])]
        self.assertEqual(checkouts.count(checkouts.SCANNER, runs, DAY),
                         {checkouts.SCANNER: 2, checkouts.ACTION: 1})

    def test_the_action_counts_its_own(self):
        runs = [("Test", [job("Fixtures", step("Run actions/checkout@x"))])]
        self.assertEqual(checkouts.count(checkouts.ACTION, runs, DAY),
                         {checkouts.SCANNER: 0, checkouts.ACTION: 1})


if __name__ == "__main__":
    unittest.main()
