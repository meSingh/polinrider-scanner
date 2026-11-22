---
title: Block a merge when the action finds PolinRider
description: "Make the PolinRider action a required check with a GitHub ruleset, so an infected push or pull request cannot merge into your default branch."
---

On its own, a failed check is a red cross that anyone with write access can
merge past. A **ruleset** turns it into a rule: a pull request whose scan
failed cannot merge, and nobody can push around it. Set it up once per
repository, or once for a whole organization.

This page assumes the action is already running on pushes and pull requests
([Set up the GitHub action](/docs/action/setup/)) and has run at least once,
so GitHub knows the name of its check.

## What blocks a merge

The check is the `scan` job of your workflow. It fails, and so blocks the
merge, when:

| The scan says | `fail-on: confirmed` (default) | `fail-on: review` |
|---|---|---|
| clean | passes | passes |
| something to review | passes | **blocks** |
| infected | **blocks** | **blocks** |
| could not run | **blocks** | **blocks** |

A scan that could not run always blocks: nothing scanned is never treated as
clean. To block on review items too, set `fail-on: review`
([Inputs](/docs/action/inputs/)).

## Set it up in the browser

1. Open the repository's **Settings**, then **Rules**, then **Rulesets**.
2. Choose **New ruleset**, then **New branch ruleset**.
3. **Ruleset name:** `PolinRider`. **Enforcement status:** Active.
4. **Bypass list:** leave it empty. Anyone listed there can merge an infected
   pull request.
5. **Target branches:** choose **Add target**, then **Include default branch**.
   Add any release branches you merge into as well.
6. Turn on these rules:
   - **Require a pull request before merging.** Changes then arrive through a
     pull request, where the scan runs before the merge.
   - **Require status checks to pass.** Choose **Add checks**, type `scan` and
     pick it. Then set its source to **GitHub Actions**, so only the workflow
     can report it and nobody can mark it green with a token.
   - **Block force pushes.**
   - **Restrict deletions.**
7. Choose **Create**.

The pull request's merge box now lists `scan` as **Required**, and the merge
button stays disabled until it passes.

## The check's name

The name to require is the job's name, not the workflow's. In the workflow
from the setup page the job is `scan`:

```yaml
jobs:
  scan:
```

- If the job has a `name:`, require that name instead.
- If you rename the job later, update the ruleset. A required check that never
  reports leaves every pull request waiting.
- A job with a `matrix` reports one check per entry, such as `scan (ubuntu-latest)`.
  Require each one.

## Set it up from the command line

The same ruleset with the [GitHub CLI](https://cli.github.com/). Replace
`OWNER/REPO`. `15368` is the GitHub Actions app, the only source the check is
accepted from.

```bash
gh api --method POST repos/OWNER/REPO/rulesets --input - <<'EOF'
{
  "name": "PolinRider",
  "target": "branch",
  "enforcement": "active",
  "bypass_actors": [],
  "conditions": {
    "ref_name": { "include": ["~DEFAULT_BRANCH"], "exclude": [] }
  },
  "rules": [
    { "type": "pull_request",
      "parameters": {
        "required_approving_review_count": 0,
        "dismiss_stale_reviews_on_push": false,
        "require_code_owner_review": false,
        "require_last_push_approval": false,
        "required_review_thread_resolution": false
      } },
    { "type": "required_status_checks",
      "parameters": {
        "strict_required_status_checks_policy": false,
        "required_status_checks": [ { "context": "scan", "integration_id": 15368 } ]
      } },
    { "type": "non_fast_forward" },
    { "type": "deletion" }
  ]
}
EOF
```

To see what now applies to the default branch:

```bash
gh api repos/OWNER/REPO/rules/branches/main --jq '.[].type'
```

## Every repository in an organization

An organization owner can create the same ruleset once under the
organization's **Settings**, **Repository**, **Rulesets**, and target **All
repositories** or a pattern. Two things to know:

- Each repository still needs the workflow. A required check from a
  repository that has no `scan` job never reports, and its pull requests
  cannot merge until the workflow is added.
- Organization rulesets depend on your GitHub plan. If the option is missing,
  set the ruleset up per repository.

## What it does not block

- **The weekly scheduled scan** blocks nothing. It runs on code that is
  already merged. When it fails, GitHub emails the person who last changed the
  workflow's schedule. Act on it as on any infected result.
- **Other branches.** A push to a branch the ruleset does not target is
  scanned and can fail, but nothing stops it. It is blocked when it tries to
  merge into a targeted branch.
- **Anyone on the bypass list.** Keep it empty, or keep it to the people who
  would be the ones cleaning up.

## Fix mode and the ruleset

In [fix mode](/docs/action/fix-mode/) the action opens a pull request that
removes the payload. That pull request is scanned like any other, and passes
once the payload is gone, so it can merge under the same ruleset. The action
never pushes to the branch it scanned, so it never needs to bypass the rules.
