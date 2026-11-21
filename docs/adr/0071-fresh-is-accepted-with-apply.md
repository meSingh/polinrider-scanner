# 0071. Accept --fresh with --apply

| | |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-11-21 |

## Context

ADR-0060 made a stopped `check` continuable with `--resume`, and gave
`--fresh` to start again without being asked. A `check --apply` moves files,
so it is never continued: it writes no progress file and always starts from
the beginning. ADR-0060 had it refuse both flags, `--fresh` included, with
"a check that moves files is never continued, it always starts from the
beginning".

The 2026-11-16 field run of 3.1.0 passed `--fresh` to every folder to be sure
each started clean. With `--apply` that was 39 instant exit-3 results and no
scan. `--fresh` asked for exactly what `--apply` already does.

## Decision

- `check --apply --fresh` is accepted. It runs exactly as `check --apply`
  does and prints nothing more. An earlier stopped check's progress file is
  left as `--apply` alone leaves it.
- `check --apply --resume` stays refused, with the same text. Continuing a
  run that moves files is what ADR-0060 rules out, and it still holds.
- `--fresh` stays refused for `clean` and the guided flow, which have no
  progress to start over from.

## Consequences

- A script can pass `--fresh` to every run, with or without `--apply`.
- A flag is accepted that changes nothing in one mode. That is the cost the
  refusal was guarding against (a flag that silently does nothing). It does
  not apply here: what `--fresh` promises, a run from the beginning, is true
  of every `--apply` run.

## Related

- ADR-0060, whose refusal of `--fresh` with `--apply` this replaces.
