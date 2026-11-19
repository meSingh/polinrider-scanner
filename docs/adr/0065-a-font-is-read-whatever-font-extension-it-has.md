# 0065. A font is read whatever font extension it has

| | |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-11-20 |

## Context

The font check read `.woff` and `.woff2` files and confirmed one whose first
bytes are not a web font's. The loaders force-pushed into GitHub repositories
from 2026-10-22 to 2026-10-26 moved the payload to `public/fonts/fa-solid-300.ttf`
and `fa-solid.lkf` (GitGuardian, 2026-10-27). The 2026-11-16 indicator review
found that a `.ttf` was matched by nothing but its name, at the review tier,
and that the task-command pattern, `node … fa-solid-NNN.woff2`, did not match
`node ./public/fonts/fa-solid-300.ttf` either.

TrueType and OpenType fonts are common in projects, and unlike web fonts they
run large: a CJK font is tens of megabytes. ADR-0045 hashes every font the
check reads, at any size.

## Decision

- The font check, local and on GitHub, reads `.ttf` and `.otf` files as well
  as `.woff` and `.woff2`, matching the extension in any case. A TrueType or
  OpenType font is real when it opens with `00 01 00 00`, `true`, `OTTO` or
  `typ1`; anything else is the existing confirmed finding, "font file is not
  a font".
- A `.ttf` or `.otf` larger than 4 MB is read for its first four bytes only
  and is not hashed. Every published fake font is under 1 MB. This amends
  ADR-0045 for these two extensions; `.woff` and `.woff2` are hashed at any
  size as before.
- `.lkf` and `.llf` are not font formats and are not read as fonts. The
  campaign's names for them are in `filenames.txt` already.
- The task-command pattern in `strong-patterns.txt` takes any font or
  fake-font extension (`woff`, `woff2`, `ttf`, `otf`, `eot`, `lkf`, `llf`)
  with or without a weight. It does not take `.js`, so `node ./fa-solid.js`
  is not matched.
- `fa-solid-[0-8]NN.ttf` stays in `filenames-review.txt`, as the `.woff2`
  entry does: a fake one is now confirmed by its bytes, and a real font
  carrying a weight Font Awesome does not ship is still worth a look.

## Consequences

- A fake `.ttf` or `.otf` is confirmed where it lies, and a task that runs
  one is confirmed in `tasks.json`.
- More files are read. Most projects ship a few fonts; the walk prunes
  `Library` and `node_modules`, so system and package fonts are not among
  them, and a large font costs four bytes.
- **A fake font larger than 4 MB that starts with a font's first bytes is not
  caught by its hash.** No published one is that large; if one is, the limit
  is the place to change.
- **Another extension the campaign moves to** (Abnormal AI saw `.orc`, with
  no full name given) is not read until it is added here.

## Related

- ADR-0045, the hashing this limits for two extensions
- ADR-0048, the review tier the `.ttf` name stays in
- ADR-0052, the content patterns the task command is one of
