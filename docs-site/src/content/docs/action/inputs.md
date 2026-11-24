---
title: PolinRider action inputs, outputs and permissions
description: "Every input and output of the PolinRider Scanner action, what fails the check, and the permissions each mode needs. Check mode is read-only."
---

## Inputs

| Input | Default | Meaning |
| --- | --- | --- |
| `mode` | `check` | `check` reads only. `fix` also removes the payload and opens a pull request |
| `path` | `.` | The directory to scan, relative to the workspace |
| `fail-on` | `confirmed` | `confirmed` fails on a confirmed finding. `review` also fails on a finding to review. `never` only reports |
| `comment` | `false` | `true` posts the result on the pull request and keeps that one comment up to date |
| `sarif-file` | empty | Also write the findings as SARIF to this path, for code scanning |
| `token` | `github.token` | Used by fix mode and `comment: true` only. The scan itself never sees a token |

## What fails the check

| Verdict | `fail-on: confirmed` | `fail-on: review` | `fail-on: never` |
| --- | --- | --- | --- |
| `clean` | passes | passes | passes |
| `review` | passes | fails | passes |
| `infected` | fails | fails | passes |
| `error`, could not run | fails | fails | fails |

The verdicts are the laptop tool's exit codes: 0 clean, 1 review, 2 infected
and 3 could not run ([Exit codes](/docs/reference/exit-codes/)). A scan that
could not run always fails, whatever `fail-on` says. Nothing scanned is never
reported as clean. In fix mode the step still fails after it opens a pull
request, because the commit it scanned is infected until that pull request is
merged.

## Outputs

| Output | Value |
| --- | --- |
| `verdict` | `clean`, `review`, `infected` or `error` |
| `confirmed` | The number of confirmed findings |
| `review` | The number of findings to review |
| `files` | The number of files scanned |
| `pr-url` | The pull request fix mode opened, or found already open |

To act on the verdict in a later step, give the step an `id`:

```yaml
      - id: polinrider
        uses: meSingh/polinrider-action@de52596b9ced60ae70152745d03fa5c36464d329 # v1.1.1
        with:
          fail-on: never
      - if: steps.polinrider.outputs.verdict == 'review'
        run: echo "PolinRider: ${{ steps.polinrider.outputs.review }} to review"
```

## Permissions per mode

| What you use | Needs |
| --- | --- |
| `mode: check` | `contents: read` |
| `comment: true` | also `pull-requests: write` |
| `sarif-file` with `upload-sarif` | also `security-events: write` |
| `mode: fix` | `contents: write` and `pull-requests: write` |

Give a job only what its mode uses. Set permissions on the job rather than
the whole workflow when one workflow has a check job and a fix job. The
`token` is passed only to the steps that comment or open a pull request,
never to the step that runs polinrider.
