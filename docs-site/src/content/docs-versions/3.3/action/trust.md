---
title: What the PolinRider action trusts, and the vendored alternative
description: "The three things the PolinRider Scanner action relies on, how to check each one yourself, and the copy of the scanner you can keep in your repository instead."
---

A scan step that downloads someone else's code on every push has the same
shape as the attack it is meant to catch. This page says exactly what the
action relies on, so you can decide whether to take that trade.

## Three things

1. **The action's repository at the SHA you pin.** A SHA cannot change, so
   what you reviewed is what runs.
2. **The polinrider release on GitHub.** Built by the scanner's release
   workflow, attested and listed in its `SHA256SUMS`.
3. **The hashes in `action.yml`.** The scanner's release workflow copies
   them from the release's `SHA256SUMS` and opens a pull request on the
   action's repository. The maintainer merges it and tags a release. No one
   writes them by hand.

The scan step itself gets no token, and runs with an empty home directory.

## Check it yourself

In a checkout of the SHA you pin:

```bash
VERSION=$(sed -n 's/.*POLINRIDER_VERSION: "\(.*\)"/\1/p' action.yml)
gh release download "v$VERSION" --repo meSingh/polinrider-scanner --pattern SHA256SUMS --pattern "polinrider-v$VERSION-linux-x86_64.tar.gz"
sha256sum -c SHA256SUMS --ignore-missing
gh attestation verify "polinrider-v$VERSION-linux-x86_64.tar.gz" --repo meSingh/polinrider-scanner
```

Then compare the hash for that archive with the one in `action.yml`.
[Verify polinrider before you run it](/docs/3.3/reference/verifying/) has more on
the release's checks.

## What it does not protect against

Someone who took over the maintainer's account could publish a new release of
the action with new hashes. Your pin stops that reaching you on its own. The
defence is Dependabot's pull request: read it before you merge it, and check
that the polinrider version it moves to is a real release.

## The vendored copy

If you would rather trust nothing fetched while the scan runs, install the
scanner into your repository and review it like your own code. It is a shell
port of the checks, so its verdict can differ from the binary's, and it
stays at the version you installed until you install it again.
[Block PolinRider in CI](/docs/3.3/guides/ci/) compares the two side by side and
shows how to install the copy.
