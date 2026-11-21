# 0070. Tell a pipe once a minute that a step is still running

| | |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-11-22 |

## Context

ADR-0059 gave a live terminal a region redrawn in place, with a count of how
far the running step has come. A pipe, a log and the report file get each
step's header as it starts, each finding as it is made and the step's time
as it ends, and nothing between.

The 2026-11-16 field run piped every folder's output to a log. Step 2 ran for
more than 20 minutes on large folders with nothing printed, which reads as a
hang.

## Decision

- Standard output, when it is not a live terminal, gets one line once a step
  has run for a minute, and then at most one a minute:
  `    still running after 2m 1s: 118,784 files read so far`. The count is the
  one the live region shows, in the same words: files listed (step 1), files
  read (step 2, "counting the earlier check" when resumed) or files hashed
  (step 3).
- The line comes with the step's own count, which arrives every 1,024 files
  and before each file hashed, so "a minute" is "about a minute". A step that
  sends no count, such as one very large file being read, sends no line.
- The report file gets no such lines. It has each step's time already, and a
  line a minute would bury the findings.
- A live terminal is unchanged.

## Consequences

- A log of a long run shows it moving.
- A log of a run of several hours holds a few hundred more lines.
- The interval is fixed. The corpus cannot test a line that takes a minute
  to appear without a setting made only for it; the unit tests in
  `screen.rs` pin the timing and the words instead.

## Related

- ADR-0059, whose pipe output this adds to.
