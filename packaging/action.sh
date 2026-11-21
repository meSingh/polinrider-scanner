#!/usr/bin/env bash
# action.sh - move the GitHub action to a new release.
#
#   packaging/action.sh VERSION SHA256SUMS [REVIEWS] < action.yml > new.yml
#
# Reads action.yml as it is in meSingh/polinrider-action and changes only the
# pinned lines of its download step: the version, the date of the newest
# indicator review this release carries, and the SHA-256 of each archive,
# taken from the release's own SHA256SUMS. Anything else edited in the
# action survives. Changes nothing else, fetches nothing, runs nothing.
#
# REVIEWS is the directory of indicator reviews at the release's tag,
# docs-site/src/content/docs/indicator-reviews beside this script by
# default. release.yml runs this from a checkout of the tag, so the date is
# the newest review in the release, never one made after it.
#
# The pinned lines look like this, one each, and every POLINRIDER_SHA256_
# line names an archive by its platform (LINUX_X86_64 is linux-x86_64):
#
#   POLINRIDER_VERSION: "3.1.0"
#   POLINRIDER_INDICATORS_REVIEWED: "2026-11-09"
#   POLINRIDER_SHA256_LINUX_X86_64: "<64 hex>"
#
# Run by bump.sh as its renderer (ADR-0062).

set -euo pipefail

die() { echo "action.sh: $*" >&2; exit 1; }

[[ $# -eq 2 || $# -eq 3 ]] || die "usage: packaging/action.sh VERSION SHA256SUMS [REVIEWS] < action.yml"
version=$1 sums=$2
reviews=${3:-"$(cd "$(dirname "$0")/.." && pwd)/docs-site/src/content/docs/indicator-reviews"}
[[ -f "$sums" ]] || die "no SHA256SUMS at $sums"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "not a final release version: $version"
[[ -d "$reviews" ]] || die "no indicator reviews at $reviews"

# The newest review, by the date its file is named for (2026-10-26-socket.md
# is a review of 26 October).
reviewed=$(find "$reviews" -maxdepth 1 -type f -name '20[0-9][0-9]-[01][0-9]-[0-3][0-9]*.md' \
  -exec basename {} \; | cut -c1-10 | sort | tail -n 1)
[[ -n "$reviewed" ]] || die "no dated review in $reviews"

awk -v version="$version" -v reviewed="$reviewed" -v sums="$sums" '
  BEGIN {
    while ((getline line < sums) > 0) {
      split(line, f, /[ \t*]+/)
      hash[f[2]] = f[1]
    }
  }
  function value(v) { sub(/"[^"]*"[ \t]*$/, "\"" v "\""); print }
  /^[ \t]+POLINRIDER_VERSION:[ \t]*"/ { versions++; value(version); next }
  /^[ \t]+POLINRIDER_INDICATORS_REVIEWED:[ \t]*"/ { dates++; value(reviewed); next }
  /^[ \t]+POLINRIDER_SHA256_[A-Z0-9_]+:[ \t]*"/ {
    key = $1; sub(/:$/, "", key); sub(/^POLINRIDER_SHA256_/, "", key)
    if (key in seen) {
      print "action.sh: " $1 " appears twice" > "/dev/stderr"; failed = 1; exit 1
    }
    seen[key] = 1
    platform = tolower(key); sub(/_/, "-", platform)
    file = "polinrider-v" version "-" platform ".tar.gz"
    if (!(file in hash)) {
      print "action.sh: " file " is not in SHA256SUMS" > "/dev/stderr"; failed = 1; exit 1
    }
    if (hash[file] !~ /^[0-9a-f]+$/ || length(hash[file]) != 64) {
      print "action.sh: the SHA256SUMS line for " file " is not a SHA-256" > "/dev/stderr"; failed = 1; exit 1
    }
    shas++
    value(hash[file]); next
  }
  { print }
  END {
    if (failed) exit 1
    if (versions != 1 || dates != 1 || shas == 0) {
      print "action.sh: expected one POLINRIDER_VERSION, one POLINRIDER_INDICATORS_REVIEWED and at least one POLINRIDER_SHA256_ line, got " versions + 0 ", " dates + 0 " and " shas + 0 > "/dev/stderr"
      exit 1
    }
  }
'
