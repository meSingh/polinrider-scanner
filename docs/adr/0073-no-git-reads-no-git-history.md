# 0073. --no-git reads no git history

| | |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-11-22 |

## Context

ADR-0068 made `check` and `clean` read each repository's git history: the
last commit, the staged files, every branch, remote branch and tag, and every
stash. On a computer that is the point. The files on disk can be clean while
the last commit still holds the payload, and one `git restore` brings it back.

The GitHub Action runs the scanner on a fresh checkout of the commit it was
triggered by. There the files on disk are that commit, so every git finding
repeats what the file checks already found. With 3.3.0 the Action's own tests
showed it: two confirmed findings became three, the SARIF upload carried the
extra one, and the fix pull request listed the history as "still carries the
payload, not changed" although merging that pull request is the fix. A
workflow that fetches every branch (`fetch-depth: 0`) would go further and
fail a clean pull request for a payload on an unrelated branch.

The Action's job is to stop the payload arriving in the commit it checks.
Reading the whole history is the job of the scan on a computer, where someone
can act on it.

## Decision

- `--no-git` reads no git history. The files on disk are checked exactly as
  before, `.git/hooks` included, since those are files on disk.
- Accepted by `check` and `clean`. Refused by the guided flow and
  `--purge-evidence`, like `--git-all`.
- Refused together with `--git-all`: one reads no history and the other reads
  all of it.
- Nothing is printed to say history was not read. Whoever passes the flag
  asked for that, and in CI nobody expects a check of one commit to read the
  rest of the repository.
- It is one of the options a stopped check must share to be continued.
- The Action passes it on every run.

## Consequences

- The Action reports what it reported with 3.2.0 for the commit it checks.
- A run with `--no-git` can call a folder clean that git would reinfect. That
  is the trade the flag names, and the machine scan does not pass it.

## Related

- ADR-0068, whose git reading this lets a caller turn off.
