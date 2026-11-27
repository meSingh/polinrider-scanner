# 0075. Total reach counts unique cloners, not clones

| | |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-11-27 |

## Context

ADR-0074 counted clones by others as GitHub's clone count less our own
workflows' checkouts. That counts every fetch. In October others made 1,656
clones of this repository, while GitHub's unique cloners, summed over the
month's days, came to 802: bots, mirrors and services that fetch the same
repository again and again weigh more than people in a count of fetches.

GitHub gives unique cloners per day. Our own checkouts collapse into very
few of them: on 2026-10-12 our workflows made 375 checkouts and GitHub
counted 86 unique cloners in all, and on 18 of 42 days our checkouts alone
outnumber the day's uniques. GitHub does not say how it tells cloners apart;
that our runs count as one cloner is inferred from these numbers.

## Decision

- `github_clones_by_others` carries `uniques`, `ours_uniques` and
  `others_uniques`: GitHub's unique cloners, one on a day our workflows
  checked the repository out and none otherwise, and the difference, never
  below zero. `others`, the fetches, stays on the event.
- `reach/monthly.csv` and the PostHog reach tiles use `others_uniques`.

## Consequences

- The total is lower and nearer to people: 2,888 all time on 2026-11-27,
  against 4,768 counted in fetches.
- Uniques are per day. Summed over a month, someone who clones on three
  days counts three times, so it is still an upper bound on people.
- If our runs count as more than one cloner a day, others are overcounted
  by the difference; if a day's only cloner was us, it reads zero.
- Amends ADR-0074, whose counting of our own checkouts stays as it is.
