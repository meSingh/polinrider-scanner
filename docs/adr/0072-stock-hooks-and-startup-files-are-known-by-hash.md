# 0072. Know stock hooks and startup files by hash, and leave generated output out of the long-line review

| | |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-11-22 |

## Context

The 2026-11-16 field report opened all 193 review items of a 700 GB drive by
hand and found none malicious. Three kinds were common and come from tools:

- 60 "active git hook" lines, every one a stock hook of git-lfs, husky 0.13.4
  or yorkie 2.0.0;
- 5 "Python startup file runs code" lines, every one coloredlogs.pth
  (sha256 dda83a85...), which turns itself on through an `exec`;
- 5 "line longer than 4000 characters" lines, in `.nuxt/eslint.config.mjs`
  and `.nuxt/types/tailwind.config.d.ts`, which Nuxt writes and rewrites.

A review line nobody can act on teaches people to skip review lines.

## Decision

- **`ioc/stock-files.txt`** lists SHA-256 hashes of files tools write the
  same way everywhere, each with a kind, `hook` or `pth`, and a label. Each
  line's source is in the file: every entry was produced in the development
  container from the tool's own release (git-lfs 2.13.3 to 3.6.1 running
  `git lfs install --local`; husky 0.13.4 and yorkie 2.0.0 installed by npm;
  coloredlogs.pth from the 10.0, 14.0 and 15.0.1 wheels). It is read with the
  other indicator files and is part of the digest a resumed check compares.
  A line in any other form is not used, as in `hashes.txt`.
- **The confirmed indicators come first.** A hook or startup file carrying
  one is confirmed whatever the list says. A stock match only keeps a file
  out of review.
- **A stock hook is counted, not reviewed:** "3 git hooks are the stock hooks
  git-lfs, husky or yorkie write". The line does not say "byte for byte":
  for husky and yorkie hooks that holds apart from the two normalised lines.
- **husky and yorkie hooks are hashed after two lines are normalised**,
  because both write the installing user's home and the package folder into
  every hook, so no single hash could match two machines:
  - `load_nvm NVM_DIR <path>` (husky) or `load_nvm <path>` (yorkie), only
    when the path is `/home/<name>/.nvm`, `/Users/<name>/.nvm` or
    `/root/.nvm`, the name of letters, digits, `.`, `_` and `-`, and not `.`
    or `..`;
  - `cd <dir>` (husky) or `cd "<dir>"` (yorkie), only when the folder is
    relative, of letters, digits, `.`, `_`, `/` and `-`, with no `..`.

  A line of either kind in any other form is hashed as it is, so the hook is
  not stock. Every other byte counts. With those characters the two lines can
  only source the user's own `~/.nvm/nvm.sh`, which nvm runs anyway, or change
  into a folder inside the project, whose `package.json` the other checks read.
- **A stock `.pth` is quiet.** Only coloredlogs.pth needs listing.
  `distutils-precedence.pth`, `_virtualenv.pth` and the `-nspkg.pth`
  namespace stubs were checked against the real files and already use none
  of the calls the startup-file check looks for.
- **A `.d.ts` file is not a build config**, whatever its name: TypeScript
  declarations are never run. It is still read as TypeScript source, so an
  indicator in it is confirmed. This applies to the GitHub check too, which
  shares the list of build-config names.
- **Under a `.nuxt` folder, the long-line review is not applied.** The
  indicators, the known hashes and the payload-shaped tail still are. Only
  `.nuxt`: `.next`, `.svelte-kit`, `.output` and `.astro` wait for a field
  sample, since each folder added is a check given up on inference.

## Consequences

- The field drive's 70 reviews of these kinds go away.
- A hook from a version not listed, or written by husky on Windows, stays a
  review line. That errs towards a look.
- The list has to be kept: a new git-lfs release that changes its hook text
  needs a new line, produced the same way, before its hooks stop being
  reviewed.

## Related

- ADR-0042, the Python startup file check, which this narrows.
- ADR-0045, the known hashes, whose format this follows.
- ADR-0047, the weak signals, the other source of review lines.
