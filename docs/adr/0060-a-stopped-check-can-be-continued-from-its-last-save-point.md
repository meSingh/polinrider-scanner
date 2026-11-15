# 0060. A stopped check can be continued from its last save point

| | |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-11-15 |

## Context

A check of a 700 GB backup drive ran for a day and was lost. ADR-0059 made a
scan keep what it finds as it goes and stop reading what cannot match, but a
check that stops still starts again from the beginning (ADR-0030), and on a
drive that size most of the time is step 2: reading every JavaScript and
TypeScript file and every file under 1 MiB. A second day spent re-reading
what the first one already read is the cost ADR-0030 accepted while scans
were short. On this drive they are not.

ADR-0030 refused checkpoints for two reasons, both still good: a state file
holding an inventory of every path on the machine, and the 1.x bug where a
second run inherited a first run's checkpoints, skipped every check and
reported clean for work it never did. This design is shaped by both.

## Decision

1. **A progress file beside the report**, `<report stem>.progress`. It is
   written whole to a `.part` file, synced and moved over the old one every
   5,000 files of step 2's pass over the project files, and when step 3
   starts. It holds what the run was (version, a digest of the indicator
   files, the folders, the options, `check` or `guide`, the report, the
   process id, the start time), where it had got to (the step, the count, the
   last two save points) and the paths of the files the pass found something
   in. A save point is one path: the pass goes through the walk's files in
   sorted order, so every file that sorts before it was done. **No inventory
   of the machine**: two paths and the finding paths. It is removed when the
   check finishes.

2. **What a continued check does again.** Step 1 (the host checks and the
   whole walk, with the folder-open tasks, build configs and git hooks read
   as it lists them), step 3, and the rest of step 2 (fonts, the propagation
   script, named files, packages, credentials) run again in full. Only the
   pass over the project files is shortened: it skips every file that sorts
   before the **older** of the two save points, 5,000 to 10,000 files before
   the stop, except a file that had a finding in the stopped run, which is
   read again. The result is made only from this run's reads: nothing is
   copied from the stopped run's findings. A file added before the save
   point since the stopped run is not read, and the report says so.

3. **Only a run that matches it can use it.** A progress file is refused,
   said and removed, and the check starts from the beginning, when it was
   written by another version, with other indicators (a new indicator must
   see the files already read), of other folders, with other options, when
   it is damaged (not whole and well formed, or not beside its own report),
   or when its report is missing. One whose report has finished is left
   over from a check that ended: nothing to continue, and it is removed. One
   whose process is still running is left alone, so two checks of the same
   folders at once never share one. This is the guard against the 1.x bug:
   a progress file belongs to one report, and a run skips only files before
   a save point a matching run wrote.

4. **Who continues, and how they are asked.**
   - `check` with `--resume` continues without asking, or says there is
     nothing to continue and starts from the beginning. `--fresh` starts
     from the beginning without a word.
   - Without either, on a live terminal it asks, and the answers are words,
     yes or no. The end of input is a no: nothing is skipped without a yes.
   - Into a pipe or a log it never asks: it starts from the beginning and
     says how to continue instead. A scheduled check is never cut short by
     an earlier one.
   - The guided check asks the same question in its own words.
   - **Not `clean`**, and not a `check --apply`. Both move or strip files,
     so what the stopped run already moved cannot be found again by a run
     that continues it, and its result could read as clean. A half-done
     clean is safer run again from the start. Neither writes a progress
     file, and both refuse `--resume` and `--fresh`.
   - `--resume`, refused since 2.0 as a removed 1.x flag, is a real flag
     again. `--state` stays refused.

5. **The report of a continued check** has a "Resumed" section: when the
   stopped check stopped, that steps 1 and 3 ran again, where step 2 read
   again from, how many files before that point were not read and that a
   file added there since was not. Step 2's count on screen says it counts
   the earlier check.

## Consequences

- **A day-long check that stops costs minutes to continue, not a day.** The
  walk and steps 1 and 3 run again; on the synthetic tree of ADR-0059 that
  is under a second of 10 seconds.
- **The walk is not saved.** If listing the drive is itself a large share of
  the time, continuing saves less. Not measured on the real drive; saving
  the file list would be the next step, and it would bring back the
  inventory ADR-0030 refused.
- **A file added before the save point since the stop is missed.** Said in
  the report, in those words.
- **Counts are to the last save point.** "After N files" and "N files before
  the stop" count to the last save point, not the exact stop, which can be up
  to 5,000 files later: there is no signal handler to note it (ADR-0059).
- **A file is written beside the report** while a check runs, and removed
  when it ends. One left by a stopped check is removed by the next check of
  the same folders, continued or not.
- **The progress file is not encrypted or hidden.** It names the files with
  findings, which the report beside it already does.
- **Not on Windows yet, partly.** The time in the question is in UTC there,
  and says so, because the local zone is asked of `/bin/date`. Whether a
  check from another process is still running is not tested there either.

## Related

- ADR-0030: no checkpoint. Amended: a check that stopped can be continued
  from a save point. Its two objections are answered in decision 1 (no
  inventory) and decision 3 (only a matching run, never a running one).
- ADR-0059: riskiest first, findings saved as found, no signal handler.
  Amended: step 2's pass saves its place, and a check can continue it.
- ADR-0025: the 1.x state directory this replaces nothing of.
