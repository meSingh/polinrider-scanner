---
title: "PolinRider action fix mode: a pull request that removes the payload"
description: "What the PolinRider Scanner action's fix mode changes, the pull request it opens, why it never commits to your branch, and each reason it opens none."
---

Fix mode removes the payload on the runner and opens a pull request with the
result. It never commits to the branch it scanned, never force-pushes and
never merges. A person reads the change and merges it.

## Turn it on

```yaml
on:
  push:
    branches: [main]
  workflow_dispatch:
permissions:
  contents: read
jobs:
  fix:
    runs-on: ubuntu-latest
    permissions:
      contents: write
      pull-requests: write
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - uses: meSingh/polinrider-action@de52596b9ced60ae70152745d03fa5c36464d329 # v1.1.1
        with:
          mode: fix
```

Keep your read-only `scan` job for pull requests, and run fix mode on the
default branch.

## What it changes

When the scan finds the payload, fix mode runs `polinrider clean --apply` on
the checkout. That cuts an appended payload out of a file and keeps the
rest, and removes a file that was nothing but the payload. Then it:

1. creates a branch `polinrider/remove-payload-<sha>` at the scanned commit
   (the first 12 characters of its SHA);
2. makes one commit on it through GitHub's API, signed by GitHub, so the
   runner needs no git credentials;
3. opens a pull request into the branch it scanned.

The pull request reads like this:

```text
Remove the PolinRider payload (2 files)

PolinRider Scanner found the payload on main and removed it.

Cut out, the rest of the file kept:
- vite.config.js

Removed, the whole file was the payload:
- .vscode/tasks.json

Not changed, for a person to look at:
- src/app.js: mentions something the campaign also uses (windowsHide)

Before merging: check every computer that worked on this repository
(https://polinriderscanner.com/docs/guides/machine/), because the payload
came from one of them and will come back from it.
```

When `clean` found a payload it would not cut out safely, such as one inside
the module or written in escape codes, the pull request says so in its own
section: "Still carries the payload, not changed because cutting it out was
not safe". Those files need a person.

The step still fails. The commit it scanned is infected until the pull
request is merged. Once it is, the next run on that branch passes.

## What it never does

- Commit to the branch it scanned, force-push or merge.
- Anything on a pull request from a fork.
- Upload the quarantine. The originals `clean` keeps are the payload. They
  stay in the runner's temporary directory, outside your workspace, which
  the runner empties after the job.

## Before the first run

- **Allow it once.** In Settings, Actions, General, tick "Allow GitHub
  Actions to create and approve pull requests". Without it GitHub refuses
  the pull request and the step says so.
- **Checks on its pull request wait for approval.** A pull request opened
  with the default token starts its workflow runs only after someone with
  write access approves them. To have them start straight away, pass a
  GitHub App token or a fine-grained token with `token:`.
- **Workflow files.** A payload in `.github/workflows/` needs a token with
  the `workflows` permission to change. The default token cannot.
- **On `pull_request`,** check out the head branch
  (`ref: ${{ github.head_ref }}`), not the merge commit, or there is no
  branch tip to start the fix from. Fix mode then opens its pull request
  into that branch.

## When it opens no pull request

The job summary says why, in one line that starts "No pull request was
opened:". The step fails either way.

| The summary says | What to do |
| --- | --- |
| this pull request comes from a fork, and the action never writes to one | Nothing in the action. The check still fails, so the pull request cannot merge until its author removes the payload |
| this run is not on a branch | Run fix mode on a push to a branch or on `workflow_dispatch`, not on a tag |
| the checkout is not the tip of the branch | On `pull_request`, check out the head branch with `ref: ${{ github.head_ref }}` |
| the scanned path is not inside the checked-out repository | Point `path` at a folder inside the checkout |
| this is a branch the action opened | Nothing. It never fixes its own fix |
| files in the checkout had already changed before the scan, so the action could not tell its changes apart | Run fix mode before any step that changes files |
| polinrider clean did not finish | Read the step's log. Run `polinrider clean --apply` on a clean computer instead |
| what it removed is not tracked by git | Nothing in the repository to change. Look at how that file got there |

If a pull request for the same commit was closed without merging, it opens
no other and links the closed one. It never reopens a fix someone turned
down. If one is already open, it links that one.

## When opening it fails

These also fail the step:

| The summary says | What to do |
| --- | --- |
| Could not create the branch ... Fix mode needs contents: write. | Give the job `contents: write` |
| Could not commit the cleaned files ... A change to a file under .github/workflows needs a token with the workflows permission. | Pass a token with the `workflows` permission |
| Could not open the pull request. It needs pull-requests: write, and the repository's Actions settings must allow GitHub Actions to create pull requests. | Give the job `pull-requests: write` and tick the setting above |
| The branch ... already exists with no pull request. Delete it and run this again. | Delete the branch, then run it again |
| fix mode needs the gh command (or jq) | Install it on the self-hosted runner |

## After you merge

The payload came from a computer. Check every computer that has worked on the
repository ([Check a computer](/docs/guides/machine/)) and change the secrets
the repository's CI can read, or it will be back.
[Order matters](/docs/guides/order/) says why.
