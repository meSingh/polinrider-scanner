# 0068. Read each local repository's history with the disk check's rules

| | |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-11-22 |

## Context

The 2026-11-16 field report checked a 932 GB drive with 3.1.0: 0 confirmed.
An independent search of the git object stores on the same drive found the
payload in 47 repositories. In 3 working clones the last commit held it while
the file on disk had been cleaned, so one `git restore` or `git reset --hard`
would put it back. In 5 of 6 working clones, 20 to 64 branch tips held it.
41 were bare repositories kept from an earlier history rewrite. The local
check never read git, and `clean`, which says it never touches git, left a
folder reporting clean that one git command would reinfect.

The GitHub check already reads every branch of a mirror through `git`
(`remote.rs`). It searches every text file for the indicator strings. On the
field drive that would confirm the owner's own detection scripts and notes,
which carry those strings and which the disk check leaves alone because it
reads them only in JavaScript, TypeScript and build configs.

## Decision

- **Every repository the walk finds is read through git, at the end of
  step 2**, in its own section, "Git repositories": `.git` folders, `.git`
  files (worktrees and submodules, by the `gitdir:` they name), and bare
  repositories, known by a `HEAD` file beside `objects` and `refs` folders
  whatever they are called. A bare repository is no longer walked as files.
- **What is read by default** is what a checkout can put back: the last
  commit of each work tree, its staged files that differ from that commit,
  every branch, remote-tracking branch and tag, and every stash (its work
  tree, index and untracked files). A repository shared by worktrees has its
  branches and stashes read once.
- **`--git-all`** (check only) also reads every commit any ref or reflog
  reaches, with the path each file had, and every object nothing reaches,
  read as JavaScript.
- **The rules are the disk check's** (`git_local::judge`): a build config or
  JavaScript source carrying an indicator, a folder-open `tasks.json` carrying
  one, a font that is not a font, a file known by its hash, a manifest naming
  a campaign package or hijacked release, a name that is an indicator.
  Detection tooling is set aside as on disk. Only confirmed findings are
  reported: weak signals, folder-open tasks without an indicator and
  payload-shaped tails in history would multiply review noise, and the
  working copy is still reviewed on disk.
- **One finding per repository**, `Kind::GitHistory`, confirmed, with each
  place and its files as detail lines: five on the console and "and N more,
  in the report", all of them in the report. Its advice: commit the cleaned
  files first, then delete or rewrite each branch and drop each stash named;
  for a bare repository, delete it or keep it offline.
- **Git is read, never changed, and never takes orders.** Every call runs
  with `GIT_DIR` set, every inherited `GIT_*` variable removed,
  `GIT_CONFIG_NOSYSTEM=1`, `GIT_CONFIG_GLOBAL=/dev/null`,
  `GIT_OPTIONAL_LOCKS=0`, `GIT_NO_REPLACE_OBJECTS=1`, `GIT_NO_LAZY_FETCH=1`,
  `-c core.fsmonitor=false` and `-c protocol.allow=never`. Only plumbing is
  used: `rev-parse`, `for-each-ref`, `diff-index --cached`, `ls-files -s`,
  `log -g`, `rev-list`, and one `cat-file --batch` per repository asked one
  object at a time. `safe.directory` is not overridden.
- **Each object is read once.** Trees are walked through `cat-file`, and what
  a subtree carries at a given path is remembered, so fifty branches on
  nearly the same tree cost little more than one.
- **Limits are said, never silent:** a repository gets two minutes for its
  default places, then a review line says how many branches were read; no
  `git` is one review line counting the repositories not read; a repository
  git refuses or cannot open is a review line with git's reason; a `.git`
  file pointing nowhere is counted as info. `--git-all` has no time limit.
- It runs with `--fs-only`, since git reads only the scanned tree and none of
  the user's git configuration, and under `clean`. This amends ADR-0031:
  `clean` still never changes git, and now reads it.

## Consequences

- A cleaned folder whose history would bring the payload back no longer
  reads clean, and a bare backup carrying it is found.
- Every scan of a folder with repositories runs git, and costs a read of each
  distinct file of every branch tip. On this repository, 66 refs come to 1,488
  distinct objects, read in well under a second.
- A repository whose default places take over two minutes is partly read, and
  says so.
- History older than the tips is read only with `--git-all`.
- The GitHub check keeps its own, wider rule. A repository that commits its
  own detection scripts could be confirmed there and not here. Aligning the
  two is a separate change.
- Without a way to run as another user, the ownership refusal is not tested.

## Related

- ADR-0031, whose "never runs git" this replaces with "never changes git".
- ADR-0051, the GitHub check of every branch, whose plumbing this follows.
- ADR-0070, which tells a pipe how far the repository count has come.
