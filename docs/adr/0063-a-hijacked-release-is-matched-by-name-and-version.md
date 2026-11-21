# 0063. A hijacked release is matched by name and version

| | |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-11-20 |

## Context

On 2026-11-03 and 2026-11-04 two publisher accounts of a real project were
used to release twelve versions of ten of its packages with the loader added
to the entry file (Greenflagged, Sonatype, OpenSSF MAL-2026-18639 to 18647 and
MAL-2026-11986). npm removed them within a day. There is no install script:
the loader runs when the package is required.

Two things in this repository could not see it:

- `bad-packages.txt` holds names, and every name in it is also matched as
  text in every file (ADR-0041). These packages are real, some published since
  2018, so carrying their names would flag every user of a clean release. The
  2026-11-16 review recorded the twelve versions as comments only.
- The walk never enters `node_modules` (ADR-0025). The review found that the
  payload line in an installed hijacked package's `lib/index.js` exits 0; the
  same file outside `node_modules` exits 2.

## Decision

- **A second list, `ioc/bad-package-versions.txt`**, one `name@version` per
  line, split at the last `@` that does not open a scope. A line that is not
  in that form stops the scan with exit 3, as a pattern that does not parse
  does. The names in it are never matched as text, anywhere.
- **A match is confirmed.** Each version is a single build npm withdrew within
  a day and a MAL record names. A lockfile that pins it resolved it, and the
  loader runs on `require`. Any other release of the same package is not
  reported at all.
- **Where the version is read.** The manifest reader, which already reads
  the four formats for names, also gives the release beside each name:
  `package-lock.json` v1 to v3, `yarn.lock` v1 and berry, `pnpm-lock.yaml` v5,
  v6 and v9, and in a `package.json` its own name and version and each
  dependency's value as written. A value is compared whole, so an exact pin
  (`13.5.31`, `=13.5.31`) equals a release and a range (`^13.5.0`) never does.
- **Inside `node_modules`, a few named reads and no walk.** The walk lists
  each `node_modules` it prunes, as it lists `.venv`. For each one the check
  reads npm's `.package-lock.json`, each listed package's own `package.json`
  (`node_modules/<name>/package.json`), and lists pnpm's `.pnpm` folder for a
  folder named `<name, / as +>@<version>`, with or without a peer suffix. The
  loader file itself is not read: the release is the proof.
- **A finding kind of its own**, because the advice differs: the package is
  real and the fix is another release, not removing it. It is a file in a
  project, not proof the payload ran here, and it is not moved on `--apply`.
- **The GitHub check** confirms a lockfile that pins a hijacked release, as it
  confirms one naming a campaign package.

## Consequences

- A project that pins or has installed one of the twelve releases is
  confirmed, with or without a committed lockfile, and its users of clean
  releases see nothing.
- About twenty file checks per `node_modules`. A home folder with 500 projects
  adds about ten thousand, against a walk that lists hundreds of thousands of
  files.
- **A range that resolved to a hijacked release is caught only through a
  lockfile or `node_modules`.** A `package.json` alone, with `^13.5.0` and no
  install, is not reported; it may never have resolved the bad build.
- **`bun.lock` and `bun.lockb` are not read**, for releases or for names.
- **A nested `node_modules` inside a package** is reached only through npm's
  `.package-lock.json`, which lists nested installs. Under yarn or pnpm without
  their lockfile, a hijacked release installed only as a nested dependency is
  not found.
- **The CI scanner (`ci/scan-workspace.sh`)** does not read this list.
- The loader inside `node_modules` is still unread, so a hijacked release not
  yet in this list is still invisible there. The prune of ADR-0025 stands; this
  is a list to keep current, as `bad-packages.txt` is.

## Related

- ADR-0025, the prune this reads around without changing
- ADR-0041, the names list this deliberately does not extend
