---
title: Commands and options
description: "Every polinrider command and option: what each one does, what it changes about a scan, what it needs and which options it cannot be combined with."
---

Every command and option `polinrider` accepts, in one place. The same list is
in the terminal: `polinrider --help` for the short form, and `man polinrider`
where the manual page is installed.

Two rules hold for every option:

- **An option a command cannot use is refused, never ignored.** The run stops
  with exit code `3` and nothing is scanned. So a typo or a wrong combination
  can never quietly give you a weaker scan than you asked for.
- **Nothing is ever deleted.** No option deletes a file you have. `--apply`
  moves files into quarantine, and `--purge-evidence` deletes only the copies
  polinrider itself made, after you type `yes`.

## Commands

| Command | What it does |
|---|---|
| `polinrider` | The guided flow. Asks what to check, one question at a time, and changes something only when you type `yes` |
| `polinrider guide` | The same guided flow, spelled out, so it can take options |
| `polinrider check ROOT...` | Scans the folders given and this computer, and reports. Changes nothing unless given `--apply` |
| `polinrider clean REPO...` | Scans the folders given, and with `--apply` cuts an appended payload out of a build config in place |
| `polinrider --purge-evidence` | Deletes the copies of GitHub repositories the guided flow kept, after a typed `yes` |

A line with folders and no command is a `check`: `polinrider ~/code` is
`polinrider check ~/code`.

## Which command takes which option

| Option | `check` | `clean` | guided flow | `--purge-evidence` |
|---|:-:|:-:|:-:|:-:|
| [`--fs-only`](#--fs-only) | yes | | | |
| [`--apply`](#--apply) | yes | yes | | |
| [`--quarantine DIR`](#--quarantine-dir) | yes | yes | yes | |
| [`--report FILE`](#--report-file) | yes | yes | yes | |
| [`--resume`](#--resume) | yes | | | |
| [`--fresh`](#--fresh) | yes | | | |
| [`--git-all`](#--git-all) | yes | | | |
| [`--no-git`](#--no-git) | yes | yes | | |
| [`--ioc DIR`](#--ioc-dir) | yes | yes | yes | |
| [`--home DIR`](#--home-dir) | yes | | yes | |
| [`--host-state DIR`](#--host-state-dir) | yes | | yes | |
| [`--forge-state DIR`](#--forge-state-dir) | | | yes | |
| [`--evidence DIR`](#--evidence-dir) | | | yes | yes |

An empty cell means the option is refused with that command.

## Options that cannot be combined

| Together | Why |
|---|---|
| `--resume` and `--fresh` | One continues an earlier check and the other starts again |
| `--apply` and `--resume` | A check that moves files is never continued. It always starts from the beginning |
| `--no-git` and `--git-all` | One reads no git history and the other reads all of it |
| `--fs-only` and `--host-state` | One says not to read the computer's state and the other supplies it |

Each is refused with exit code `3` and a line that says why.

## What to scan

### `--fs-only`

**Only the folders given, nothing of this computer.** For a backup drive, a
mounted disk image or a copy of someone else's files.

- **Skips:** running processes, live connections, the crontab, what starts by
  itself, shell startup files, editor and app startup files, and the home
  folder of the user running the check.
- **Still reads:** everything inside the folders given, git history included.
- **With `--home DIR`:** that folder is read as the disk's home folder, for a
  drive that holds one. Without `--home`, no home folder is read at all.
- **Cannot be used with:** `--host-state`.
- **Caveat:** a clean result says nothing about the computer that ran the
  check. Check that computer separately, without `--fs-only`.

### `--home DIR`

**Which home folder to check.** The editor extensions, credential files,
`.ssh` and shell startup files under it are read.

- **Default:** the home folder of the user running polinrider. Under
  `--fs-only`, no home folder.
- **Use it for:** a drive or image that holds a user's home folder, or a check
  run as one user of another user's files.
- **Caveat:** quarantine folders and reports go into the home folder by
  default, and `--home` moves that too. Pass `--quarantine` and `--report` if
  you want them somewhere else.

### `--host-state DIR`

**Read the processes, sockets and crontab from files in `DIR`, not from this
computer.** For checking what was collected on another computer, and for
tests.

- **Cannot be used with:** `--fs-only`.
- **Needs:** `DIR` to exist and be a folder.

## Git history

By default each git repository the scan finds is read as well: the last
commit, the staged files, every branch, remote branch and tag, and every stash.
A payload git still holds is confirmed even when the files on disk are clean,
because one `git restore` would put it back. Git is read, never changed. Each
repository gets at most two minutes.

### `--git-all`

**Also read every earlier commit and every object git keeps.**

- **Effect:** finds a payload in a commit no branch points at any more, and
  in objects nothing reaches.
- **Cost:** much slower on a large repository. There is no time limit with it.
- **Command:** `check` only.
- **Cannot be used with:** `--no-git`.

### `--no-git`

**Read no git history at all.** The files on disk are checked exactly as
before, git hooks included, since those are files on disk.

- **Use it for:** a fresh checkout of one commit, as in CI, where the files on
  disk are the last commit. The GitHub action passes it on every run.
- **Commands:** `check` and `clean`.
- **Cannot be used with:** `--git-all`.
- **Caveat:** a folder can come back clean while git would put the payload
  back. Never use it on a computer you are cleaning.

## Changing things

### `--apply`

**Act on what is confirmed.** Without it, every command only reports.

- **With `check`:** moves each confirmed file into quarantine. It never edits
  a file.
- **With `clean`:** also cuts a payload appended to a build config out of the
  file in place, and keeps the infected original in quarantine. Your own
  uncommitted changes to that file survive.
- **Never:** deletes a file, changes git, or changes a crontab line, shell
  startup file, PowerShell profile, Run entry or scheduled task. Those
  findings carry the command that removes them.
- **Cannot be used with:** `--resume`. `--fresh` is accepted and changes
  nothing, since a run that moves files always starts from the beginning.
- **Checked before the scan:** the quarantine folder's parent must exist.

### `--quarantine DIR`

**Where moved files go.**

- **Default:** a new folder in the home folder,
  `polinrider-quarantine-<date and time>`.
- **Holds:** the files, a `manifest.tsv` of where each came from, and a
  `RESTORE.txt` that says how to put them back.
- **Caveat:** never put it inside a folder you are scanning or inside a git
  checkout. A quarantined payload there is one `git add` away from being
  published.

## The report

### `--report FILE`

**Where the full report is written.**

- **Default:** `polinrider-report-<date and time>.txt` in the home folder.
- **Written as the scan goes,** so a scan that is stopped or loses power keeps
  what it found. Until the scan finishes, the first line says it is still in
  progress.

## Long scans

A `check` saves its place every 5,000 files of step 2, in a small file beside
the report. Run the same command again and it asks whether to continue.

### `--resume`

**Continue a stopped check without asking.**

- **Continues only** when the version, indicators, folders and options are the
  same, `--no-git` and `--git-all` included. Otherwise it starts again and says
  why.
- **Command:** `check` only.
- **Cannot be used with:** `--fresh` or `--apply`.

### `--fresh`

**Start again without asking,** even if a stopped check could continue.

- **Command:** `check` only.
- **Cannot be used with:** `--resume`.
- **Into a pipe or a log** there is no one to ask, so a check starts again
  anyway and says how to continue.

## Indicators

### `--ioc DIR`

**Which indicator set to use.**

- **Default:** the `ioc` folder beside the binary, or `share/polinrider/ioc`
  where a package manager installed it.
- **Needs:** `DIR/strong.txt`. Without it the run stops with exit code `3`.
- **Caveat:** a pattern that cannot be read also stops the run with exit code
  `3`, before any file is read. A broken set never gives a clean result.

## The guided flow and GitHub

### `--evidence DIR`

**Where the guided flow keeps its copies of GitHub repositories** while it
checks them.

- **Default:** `polinrider-evidence` in the system's temporary folder.
- **Refused** inside a git checkout, so a copy of an infected repository can
  never be committed by accident.

### `--purge-evidence`

**Delete those copies,** after a typed `yes`. Nothing else is deleted.

- **Takes:** `--evidence DIR` when the copies are not in the default place.
- **Refused with** `check`, `clean` and `guide`. It is a command of its own.

### `--forge-state DIR`

**Read GitHub's answers from files in `DIR`, not from GitHub.** For tests and
for replaying a check. Guided flow only.

## Information

| Option | Prints |
|---|---|
| `-V`, `--version` | The version, the commit it was built from and the indicator counts |
| `-h`, `--help` | The short list of commands and options |

## Options that are refused on purpose

| Option | Why |
|---|---|
| `--jobs` | Parallel hashing is not built yet |
| `--background` | Detaching is not built yet |
| `--state` | Removed in 2.0, which does not checkpoint a scan this way. `--resume` continues a stopped check |

## Exit codes

`0` clean, `1` review items only, `2` a confirmed indicator, `3` the scan
could not run, which includes every refused option.
[Exit codes](/docs/reference/exit-codes/) has what to do after each.
