# 0064. Editor and app startup files are read at their install paths

| | |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-11-20 |

## Context

The November 2026 second stage keeps the payload alive outside the
repository. Besides npm's `lib/cli.js` (ADR-0042), it writes a copy of the
loader into the startup files of VS Code, Cursor, Antigravity, Discord and
GitHub Desktop, leaves `.inz.cjs`, `.inz.orig` and `.inz.tmp` files beside
them, and marks what it writes with markers that `strong.txt` has carried
since the 2026-11-16 review. Nothing read those files: the walk never enters
`/Applications` or `AppData`, and the markers matched nothing on a machine the
stage had run on.

Greenflagged's report ("@adminide-stack npm compromise", 2026-11-03, updated
2026-11-04) names the files, from the implant's own string table:

- `out/main.js` for VS Code and Cursor;
- `node_modules/@vscode/deviceid/dist/index.js` for VS Code, Cursor and
  Antigravity;
- `modules/discord_desktop_core/index.js` for Discord;
- `main.js` for GitHub Desktop.

It gives the full macOS paths, the Linux directories (`/usr/share/code`,
`/usr/share/cursor`, `/usr/share/antigravity`, `/usr/share/discord`,
`/usr/share/GitHub Desktop`) and, on Windows, `Programs\Microsoft VS Code`,
`Programs\Antigravity` and Discord's `discord_desktop_core-1` folder. It does
not say how code is placed in the editor files: no padding count, nothing on
whether it is prepended, appended or a `require` of the `.inz.cjs`. Only npm's
patch is described (200 spaces, then `/*M260630A*/`). GitGuardian has no file
list.

## Decision

A check of its own, "Editor and app startup files", in step 1 after npm's
program files, with the same shape: install places per platform under the
home directory or the host's system root (so the corpus supplies them as
files, ADR-0029), a few named files in each, and skipped under `--fs-only`.

- **Places.** Greenflagged's, with the file added where it gives only the
  directory. Added by this tool and not in the source: `~/Applications` on
  macOS; `/usr/lib/github-desktop` and `~/.config/discord/*/modules/…` on
  Linux (the packed app is in `/usr/share/discord`, the module it loads is
  under `~/.config`); on Windows the newer versioned VS Code folder, and
  Cursor and GitHub Desktop at their installers' standard places, which the
  source does not cover. `discord_desktop_core-1` is read as
  `discord_desktop_core-*`. VS Code Insiders, Program Files, Snap, Flatpak and
  AppImage installs are not read: the source does not name them, and Snap and
  AppImage are read-only images.
- **Confirmed**: a startup file holding a confirmed indicator, or a file
  beside it matching `filenames.txt` (`*.inz.cjs`). The advice is to quit the
  app and install it again.
- **Review**: a file beside it matching `filenames-review.txt` (`*.inz.orig`,
  `*.inz.tmp`); a startup file with code after a long run of blank space; a
  Discord `index.js` over 1 KB, where Discord ships one line. **These last two
  rules are this tool's inference**, from how the campaign hides a payload
  elsewhere and from how Discord ships the file. No source describes them.
- **`--apply` moves a confirmed file into quarantine**, as for npm's files.
  The app then does not start until it is installed again, which the advice
  says to do anyway.
- A finding kind of its own: it proves the payload ran on this machine.

## Consequences

- A machine the second stage ran on is confirmed by the files it patched,
  after the repository is cleaned and the package removed.
- **The padding and size rules may be wrong in either direction.** Checked
  against the real Linux packages of VS Code 1.147.0 and Cursor 3.30.14,
  where neither fired. They have not been seen against a patched file.
- **Current VS Code packs `@vscode/deviceid` inside `node_modules.asar`.** On
  VS Code 1.147.0 for Linux there is no loose `dist/index.js` to read; one is
  read only if it is there. Cursor 3.30.14 ships it loose.
- **An install somewhere else**, a portable copy or a custom prefix, is not
  read, and neither is any app the implant may add later.
- Moving `out/main.js` leaves the editor unable to start until reinstalled.

## Related

- ADR-0029, the system root that lets the corpus supply these places
- ADR-0042, npm's program files, the check this one sits beside
- ADR-0048, the review tier the `.inz.orig` and `.inz.tmp` names are in
