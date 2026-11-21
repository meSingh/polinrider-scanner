# 0069. Under --fs-only, read a home only when it is named

| | |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-11-22 |

## Context

`--fs-only` is documented as "only the checks that read the filesystem being
scanned". The home directory defaults to `$HOME`, and three checks read under
it whatever the flag said: the implant's install paths, `~/.ssh/authorized_keys`
(ADR-0049 chose that, "also with --fs-only on a disk") and the Python user
site. Two implant paths are written from the root, `/tmp/0001.dat` and
`/tmp/tmp7A863DD1.tmp`, and were read from this machine's root even when a
backup's home was named. The git hooks section printed `[ok] no global
core.hooksPath` without reading anything.

The 2026-11-16 field report ran `check --fs-only` on an external drive and
found this machine's `authorized_keys` in the report.

## Decision

- Under `--fs-only` with no `--home`, nothing under `$HOME` is read. The
  sections say so: "Second-stage implant: install paths skipped, --fs-only",
  "SSH keys that let someone in: skipped, --fs-only", and in the Python
  section "Python installed in the home folder: skipped, --fs-only".
- Under `--fs-only --home DIR`, that home is read. The operator named it as
  part of the disk being checked, which is the case ADR-0049 meant.
- Paths from the root, such as `/tmp/...`, are never read without a host to
  read: they are this machine's, not the disk's. The implant line then says
  "(--fs-only: /tmp and the process table not read)".
- The global `core.hooksPath` line under `--fs-only` is
  "global core.hooksPath: skipped, --fs-only", an info line, not an ok.

## Consequences

- `--fs-only` on an external drive reads only that drive, and says what it
  left out.
- `check --fs-only ~` on one's own machine no longer looks for the implant in
  the home or reads `authorized_keys`. Without `--fs-only` it does, and with
  `--home ~` it does again. That is a narrower default for anyone who used
  `--fs-only` as "skip the slow host checks".
- The implant lines name `/tmp` because both root paths in `implant-paths.txt`
  are under it. A root path elsewhere would need the line changed.

## Related

- ADR-0049, whose "also with --fs-only on a disk" this narrows to a home
  named with `--home`.
- ADR-0029, the boundary host state is read through.
