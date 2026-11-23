---
title: Block PolinRider in CI
description: Scan every push and pull request for PolinRider, with a GitHub action pinned by hash or with a copy of the scanner kept in your own repository.
---

> **Being migrated.** The full guide for the vendored copy currently lives in
> [`ci/README.md`](https://github.com/meSingh/polinrider-scanner/blob/main/ci/README.md) and is authoritative until this page
> replaces it.

Two ways, with different things to trust. Pick one per repository.

| | The action | The vendored copy |
|---|---|---|
| What runs | The released `polinrider` binary, the engine you run on a laptop | A shell port of the checks, kept in your repository |
| What you trust | `meSingh/polinrider-action` at the commit you pin, the scanner's release on GitHub and the SHA-256 of each archive written into the action | Only the files you copied in and reviewed |
| Fetched at scan time | One release archive, refused unless its hash matches | Nothing |
| Staying current | Dependabot moves the pin after each release | Re-run the installer in each repository |
| Removing the payload | `mode: fix` opens a pull request | By hand, or `polinrider clean` on a clean computer |

## The action

Add `.github/workflows/polinrider.yml`:

```yaml
name: PolinRider
on:
  push:
  pull_request:
  schedule:
    - cron: "17 3 * * 1"
permissions:
  contents: read
jobs:
  scan:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - uses: meSingh/polinrider-action@cd105ea591bb6d431aae82c316e2c44409e35d48 # v1.1.0
```

Then make `scan` a required check and let Dependabot move the pin.

It runs the released `polinrider` binary on the checkout and fails the check
on a confirmed finding. `mode: fix` opens a pull request that removes the
payload; it never commits to the branch it scanned. Linux runners need
`ubuntu-24.04` or later.

- [Set up the GitHub action](/docs/action/setup/): the pin, Dependabot, a
  required check, the weekly scan
- [Inputs, outputs and permissions](/docs/action/inputs/)
- [Fix mode](/docs/action/fix-mode/)
- [Reading the results](/docs/action/results/)
- [What the action trusts](/docs/action/trust/)

## The vendored copy

A scanner you copy into your own repository, so every push is checked with no
third-party action and nothing leaves your infrastructure.

```bash
./ci/install-workflow.sh /path/to/your/repo
```

This vendors the scanner and a workflow into the target repository. It runs on
every push and pull request, and fails the check when a confirmed indicator is
found.

It reads indicators written in `\uXXXX`, `\u{...}` and `\xNN` escapes as well
as plain ones, and reads `package.json` and the three lockfile formats for the
packages they install, so a campaign name counts only as a whole dependency
name. It uses `perl` and `jq` where they are installed, as they are on GitHub's
hosted runners, and says in its output when either is missing.

:::caution[Update a copy vendored from 2.1.0 or earlier]
Those copies match package names as plain substrings and do not decode escape
codes. They miss the escaped loader family and report a project that depends
on a longer package as infected. Re-run the installer with `--force`:

```bash
./ci/install-workflow.sh /path/to/your/repo --force
```
:::

Because the scanner contains the indicator strings it searches for, it will
match itself. The triage filter separates those matches by path; see
[false positives](/docs/reference/false-positives/).
