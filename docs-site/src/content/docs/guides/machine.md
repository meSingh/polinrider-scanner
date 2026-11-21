---
title: Check a computer for PolinRider
description: "Check macOS, Linux or Windows for PolinRider with one read-only program, before you touch GitHub: infected files, persistence and an installed implant."
---

> The machine check is the `polinrider` binary. macOS, Linux and Windows are
> checked by the same code.

**Read-only unless you say otherwise.** Check every machine that has touched
the affected repositories, before you touch GitHub at all.

```bash
polinrider
```

That is the guided flow: it asks what to check, one question at a time, and
changes nothing unless you type `yes`. Choose `computer`.

Without the questions:

```bash
polinrider check ~/code
```

A check of this computer and of that folder. It moves nothing. Add `--apply`
to move confirmed artifacts into quarantine. `--fs-only` checks the folder
alone and reads nothing of the machine, which is what you want for a backup
drive or a mounted image. It reads no home folder either, unless you name one
with `--home`.

## What it looks at

The `MicrosoftSystem64` implant first, malware from the same DPRK-linked group
that this tool checks for alongside PolinRider
([why](/threat/#related-malware-microsoftsystem64)):
its files, its process, and on Windows its scheduled task and Run entry. Then editor extensions, editor tasks that run on
folder open, build configs, fake fonts (`.woff`, `.woff2`, `.ttf` and `.otf`),
the script the payload spreads with, known-bad packages and hijacked releases of
real packages, Python startup files, what starts by itself, shell startup files
and PowerShell profiles, git hooks, npm configuration and npm's own program
files, the startup files of editors and apps, interpreters running inline code
and live connections. Last,
an inventory of the keys and credential files that would have to be changed if
anything else is confirmed. They are counted and named, and never opened.

"What starts by itself" depends on the system:

| System | Read |
|---|---|
| Linux | systemd units, autostart entries, the user crontab, system cron |
| macOS | LaunchAgents and LaunchDaemons, the user crontab |
| Windows | registry Run and RunOnce values, both Startup folders, scheduled tasks |

Two kinds of persistence live inside a tool rather than beside it, and both
survive cleaning every repository:

- **Python startup files.** Python runs the `import` lines of every `.pth`
  file in `site-packages` each time it starts. These are read in each
  project's `.venv`, in your own Python installations and in Python installed
  for every user. What packaging tools write there is ignored; a line that
  decodes, downloads or starts a process is listed for review.
- **npm itself.** The campaign has been seen replacing npm's `lib/cli.js` so
  that every `npm` and `npx` command starts the payload again. npm's entry
  files are read in each installation in the usual places, including version
  managers. If one is confirmed, reinstall Node.js before running npm again.
- **Editors and apps.** The campaign's second stage also changes the startup
  files of VS Code, Cursor, Antigravity, Discord and GitHub Desktop, so the
  payload starts each time one of them opens. Those files are read where each
  app installs itself. A campaign marker in one, or the loader copy the
  campaign leaves beside it, is confirmed: quit the app and install it again
  from its maker's site. Leftover backup files, code hidden after a long run of
  blank space and an oversized Discord startup file are listed for review.
  Skipped with `--fs-only`.

A hijacked release is a real package whose maintainer was compromised for a
single version. Only that version is reported, from the lockfile or from the
installed copy in `node_modules`, and the advice is to change to any other
release and reinstall. Other releases of the same package are not reported.

## Git history

Cleaning the files on disk is not enough when git still has a copy. A
`git restore`, `reset`, `checkout` or `stash pop` puts the payload back. So
each git repository the check finds is read as well: the last commit, the
staged files, every branch and tag, and every stash. Bare repositories, such
as a backup kept from a history rewrite, are read the same way.

- A payload that git still holds is confirmed, and the report names each place:
  the last commit, a branch, a tag or a stash, and the file.
- The advice is to commit the cleaned files before any restore, reset or
  checkout, then delete or rewrite each branch and stash named. Delete a bare
  backup that holds the payload, or keep it offline and labelled.
- Git is read, never changed, and a repository's own git settings cannot make
  git run a program during the check.
- Each repository gets at most two minutes; one that takes longer is listed
  for review. `--git-all` also reads every earlier commit and object, with no
  time limit.
- If git is not installed, the report says the history was not read. It is
  never passed over in silence.

## What changes, and when

Nothing is ever deleted.

- `polinrider check --apply` moves confirmed artifacts into a timestamped
  quarantine directory with a manifest and instructions for putting them back.
  It never edits a file.
- `polinrider clean DIR --apply` does that and one thing more: where the
  payload was appended to a build config, it cuts the payload out in place and
  keeps the infected original in quarantine. Your own uncommitted work in that
  file survives. It never changes git, and it says when git still holds the
  payload.
- A crontab line, a shell startup file, a PowerShell profile, a Run entry and
  a scheduled task are never changed for you. The finding carries the command
  that removes it.

## Large disks

`node_modules`, `.git`, caches and trash are never walked. Malicious packages
are caught by name from manifests and lockfiles, and the payload lives in the
project's own files rather than inside a dependency. Each manifest and lockfile
is read for the packages it installs, and each name is compared whole, so a
package whose name merely starts with a campaign one is not reported. One that
cannot be read is matched as text and listed for review.

A check goes in three steps, riskiest first:

1. **Things that run by themselves.** The implant, what starts by itself,
   editor tasks, build configs and git hooks, read as the walk meets them. A
   drive that takes an hour to list says what it found in the first minute.
2. **Project files.** Every JavaScript and TypeScript file and every small
   file, for the payload and for weak signals. On a large drive this is the
   long step.
3. **Large files against known hashes.** Only files that start like a program
   are hashed. Every known hash of that size is a program, so the others
   cannot match.

Each finding is shown and written to the report the moment it is found. The
report is saved to the disk line by line, so a check that is stopped or loses
power keeps what it found. Until the check finishes, the report's first line
says it is in progress and has no result. While a check runs the computer is
kept from sleeping, on macOS and on Linux with systemd; not yet on Windows.

### Continuing a check that stopped

A check saves its place every 5,000 files in step 2, in a small file beside
the report. Run the same command again and it asks whether to continue from
where it stopped. Steps 1 and 3 run again in full. Step 2 goes back a little
before the stop, and every file that had a finding is read again, so the
result comes only from this run.

`--resume` continues without asking and `--fresh` starts again. Into a pipe it
never asks: it starts again and says how to continue.

A check is continued only when it was made by the same version, with the same
indicators, folders and options. Otherwise it starts from the beginning and
says why. `clean` and `check --apply` always start from the beginning.

Into a pipe or a log, a long step prints a line about once a minute to say it
is still running and how far it has come.

## Exit codes

`0` clean against today's indicators, `1` something needs a human look, `2` a
confirmed indicator, `3` the check could not run. `3` says nothing about
whether you are infected.
