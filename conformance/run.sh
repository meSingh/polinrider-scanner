#!/usr/bin/env bash
# run.sh - the conformance corpus. The specification 2.0.0 is measured against.
#
# Each case in cases/ declares a filesystem to build and exactly what must come
# back. It was written so that two implementations had to give the same
# answers: the shell machine check and the Rust binary. The port finished when
# they agreed, and the shell one has since been removed, so this now holds the
# one engine to what was agreed.
#
# Usage:
#   ./conformance/run.sh                   # every case
#   ./conformance/run.sh --case fake-font  # one case, with full output on failure
#   ./conformance/run.sh --diff            # print the run output for every case
#
# Build the binary first: cargo build --release. --impl rust is still accepted,
# because scripts written when there were two say it.
#
# Exit 0 all passed, 1 a case failed, 3 the harness could not run.
#
# NOTE ON FIXTURES. No case file contains a payload. Cases write {{STRONG}},
# {{STRONG_U}} (the same entry in \uXXXX escapes), {{WEAK}} or {{BADPKG}} and
# this runner substitutes a real entry from ioc/ at build time. That keeps
# working malware strings out of the repository, and means the corpus cannot
# drift away from the indicator set it is testing.
#
# HOST CASES. A case with a "host" key describes a machine as well as a tree:
# its process table, sockets, crontab and system directories, as files. They
# run without --fs-only and with --host-state pointing at those files.
#
# GUIDE CASES. A case with "command": "guide" runs the guided flow and feeds it
# the case's "stdin", one answer per line. {{TREE}} in an answer is the
# fixture tree, so a case can type a path it could not know in advance.
#
# GITHUB CASES. A case with a "forge" key describes repositories on a pretend
# GitHub: owners, their repositories, branches and files. The runner builds
# them as real git repositories and hands the directory to the guided flow
# with --forge-state. "forge_files" adds plain files beside them: who is
# signed in, the list of organizations, a marker that gh is not installed.
# A repository's "attacks" are pushes made after it existed, each recorded as
# GitHub would record it. Afterwards every repository must be exactly as it
# was, unless "expect.forge_after" names it and says what it should hold: a
# case that pushes has to say so, and say what the push should have done.
#
# CLEAN CASES. A case with "command": "clean" runs the clean command, which
# strips an appended payload out of a build config in place.
#
# KNOWN-HASH CASES. A published file is matched by its SHA-256, and its bytes
# cannot be committed. A case's "known_hashes" lists fixture files, each with
# a label: the runner copies ioc/ to the case's own directory, adds the hash
# of each fixture to that copy's hashes.txt, and runs with --ioc pointing at
# it. ioc/ itself is never written. What is tested is that a file the checks
# read is compared by its hash, at any size (ADR-0045).

set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"

IMPL="rust"; ONLY=""; SHOW_DIFF=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --impl) IMPL="$2"; shift 2 ;;
    --case) ONLY="$2"; shift 2 ;;
    --diff) SHOW_DIFF=1; shift ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 3 ;;
  esac
done

command -v jq >/dev/null 2>&1 || { echo "conformance needs jq" >&2; exit 3; }

# --- the indicators the fixtures are built from ----------------------------
pick() { sed -e '/^#/d' -e '/^$/d' "$ROOT/ioc/$1" | sed -n "${2:-1}p"; }
STRONG="$(pick strong.txt 1)"
WEAK="$(pick weak.txt 1)"
BADPKG="$(pick bad-packages.txt 1)"
[[ -n "$STRONG" && -n "$BADPKG" ]] || { echo "indicator set is empty or unreadable" >&2; exit 3; }
# The same indicator with every character written as a \uXXXX escape, the way
# the 0xa322 loader family writes its strings (ADR-0040). Built here from
# ioc/, like the rest, so no escaped payload is committed either.
STRONG_U=""
for ((u_at = 0; u_at < ${#STRONG}; u_at++)); do
  printf -v u_hex '\\u%04x' "'${STRONG:u_at:1}"
  STRONG_U+="$u_hex"
done
# sed reads a backslash in a replacement as an escape of its own.
STRONG_U_SED="$(printf '%s' "$STRONG_U" | sed 's/\\/\\\\/g')"
# For the host cases: an implant process name, the same name as the Linux
# kernel reports it (15 bytes, no more), and a campaign address.
IMPLANT="$(pick implant-names.txt 1)"
IMPLANT_CUT="${IMPLANT:0:15}"
NETIP="$(sed -e '/^#/d' -e '/^$/d' "$ROOT/ioc/network.txt" | grep -E '^[0-9]+(\.[0-9]+){3}$' | sed -n 1p)"
[[ -n "$IMPLANT" && -n "$NETIP" ]] || { echo "implant-names.txt or network.txt is empty or unreadable" >&2; exit 3; }
# The cut-name case is only a test if the name is longer than the cut. If the
# first entry ever stops being, fail here rather than pass a case that proves
# nothing.
[[ ${#IMPLANT} -gt 15 ]] || { echo "the first implant name is not longer than 15 bytes; the kernel-truncation case needs one that is" >&2; exit 3; }
# The campaign hides the payload behind roughly 280 spaces of padding.
PAD="$(printf '%280s' '')"

# --- implementation adapters ------------------------------------------------
# Each takes: <report> <state> <quarantine|""> <root...>
# and prints the run output. The exit code is the implementation's.
impl_rust() {
  local report="$1" state="$2" qdir="$3"; shift 3
  # CARGO_TARGET_DIR moves the build output, which the sandbox does because
  # the repository is mounted read-only. Hardcoding $ROOT/target meant every
  # case failed in the sandbox while passing on the host.
  local bin="${CARGO_TARGET_DIR:-$ROOT/target}/release/polinrider"
  [[ -x "$bin" ]] || {
    echo "conformance: no binary at $bin. Build it: cargo build --release" >&2
    return 3
  }
  # $state belongs to the shell engine. Discarded here, at the top: putting
  # it after the run made ":" the last command, so rc captured its exit status
  # instead of the binary's and every case read as exit 0.
  : "$state"
  # No --state flag either: 2.0 has no checkpointing (ADR-0030) and the Rust
  # engine refuses the flag rather than ignoring it. --home is explicit so the scan reads
  # the fixture's home rather than the runner's.
  local args
  if [[ "$COMMAND" == "guide" ]]; then
    # The guided flow takes no roots and no --apply: it asks. The answers come
    # from the case. The quarantine is always named, because whether anything
    # goes into it is decided by an answer and not by a flag.
    args=(guide --home "$FAKE_HOME" --report "$report" --quarantine "$(dirname "$report")/quarantine")
    [[ -n "$IOC_DIR" ]] && args+=(--ioc "$IOC_DIR")
    [[ -n "$HOST_STATE" ]] && args+=(--host-state "$HOST_STATE")
    # Copies of repositories go beside the report, which is outside every
    # checkout: the engine refuses anywhere that is not.
    [[ -n "$FORGE_STATE" ]] && args+=(--forge-state "$FORGE_STATE" --evidence "$(dirname "$report")/evidence")
    jq -r '.stdin // [] | .[]' "$CASE_FILE" | sed -e "s|{{TREE}}|$TREE|g" | "$bin" "${args[@]}" 2>&1
    return "${PIPESTATUS[2]}"
  fi
  if [[ "$COMMAND" == "clean" ]]; then
    # clean reads the roots and nothing else: no home, no host, and it refuses
    # the flags that would say otherwise.
    args=(clean)
  else
    # "home_flag": false runs without --home, with the fixture home as $HOME:
    # the home is then this machine's, which --fs-only must not read
    # (ADR-0069). Always inside $(...), so HOME is set for this run only.
    if [[ "$HOME_FLAG" == "false" ]]; then
      export HOME="$FAKE_HOME"
      args=(check)
    else
      args=(check --home "$FAKE_HOME")
    fi
    # A host case supplies the machine; every other case reads no host state.
    if [[ -n "$HOST_STATE" ]]; then args+=(--host-state "$HOST_STATE"); else args+=(--fs-only); fi
  fi
  # An empty report path is a run left to choose its own: the home directory.
  [[ -n "$report" ]] && args+=(--report "$report")
  [[ -n "$qdir" ]] && args+=(--apply --quarantine "$qdir")
  [[ -n "$IOC_DIR" ]] && args+=(--ioc "$IOC_DIR")
  # A second run of a case (ADR-0060) can add flags of its own.
  args+=(${EXTRA_ARGS[@]+"${EXTRA_ARGS[@]}"})
  if [[ -n "$KILL_SIGNAL" ]]; then
    # Killed part way, as a closed laptop lid or a kill would: once the
    # report holds the line the case names, the signal is sent. The binary
    # catches none (ADR-0059), so the exit status is the signal's.
    local log pid krc watched="$report"
    # Or once the progress file beside it holds the line (ADR-0060).
    [[ "$KILL_IN" == "progress" ]] && watched="${report%.*}.progress"
    log="$(dirname "$report")/console.txt"
    "$bin" "${args[@]}" "$@" > "$log" 2>&1 &
    pid=$!
    until grep -qF "$KILL_WHEN" "$watched" 2>/dev/null; do
      kill -0 "$pid" 2>/dev/null || break
      sleep 0.02
    done
    kill -s "$KILL_SIGNAL" "$pid" 2>/dev/null
    wait "$pid"
    krc=$?
    cat "$log"
    return "$krc"
  fi
  if [[ "$NO_GIT" == "true" ]]; then
    # No git on the way: the binary is named by its path, so only git is
    # missing (ADR-0068).
    PATH=/nonexistent "$bin" "${args[@]}" "$@" 2>&1
    return
  fi
  "$bin" "${args[@]}" "$@" 2>&1
}

case "$IMPL" in
  rust) ;;
  shell) echo "the shell implementation was removed in 2.0: there is one engine now" >&2; exit 3 ;;
  *) echo "unknown implementation: $IMPL" >&2; exit 3 ;;
esac

# --- helpers ----------------------------------------------------------------
# A content hash of every file under a directory, so "did the run change what
# it was reading" is a question with an exact answer.
snapshot() {
  ( cd "$1" 2>/dev/null && find . -type f -print0 2>/dev/null \
      | LC_ALL=C sort -z \
      | xargs -0 shasum -a 256 2>/dev/null ) || true
}

subst() { printf '%s' "$1" \
  | sed -e "s|{{STRONG_U}}|$STRONG_U_SED|g" \
        -e "s|{{STRONG}}|$STRONG|g" -e "s|{{WEAK}}|$WEAK|g" \
        -e "s|{{BADPKG}}|$BADPKG|g" -e "s|{{PAD}}|$PAD|g" \
        -e "s|{{IMPLANT_CUT}}|$IMPLANT_CUT|g" -e "s|{{IMPLANT}}|$IMPLANT|g" \
        -e "s|{{NETIP}}|$NETIP|g"; }

# build_files <case file> <key> <destination>: write every entry of one of the
# case's file maps under a directory.
build_files() {
  local cf="$1" key="$2" dest="$3" path content
  while IFS= read -r path; do
    content="$(jq -r --arg k "$key" --arg p "$path" '.[$k][$p]' "$cf")"
    mkdir -p "$dest/$(dirname "$path")"
    subst "$content" > "$dest/$path"
  done < <(jq -r --arg k "$key" '.[$k] // {} | keys[]' "$cf")
}

# build_files_exact <case file> <key> <destination>: the same, byte for byte.
# build_files loses the newlines a file ends with, which a file known by its
# hash cannot spare.
build_files_exact() {
  local cf="$1" key="$2" dest="$3" path content
  while IFS= read -r path; do
    content="$(jq -j --arg k "$key" --arg p "$path" '.[$k][$p]' "$cf"; printf x)"
    content="${content%x}"
    mkdir -p "$dest/$(dirname "$path")"
    subst "$content" > "$dest/$path"
  done < <(jq -r --arg k "$key" '.[$k] // {} | keys[]' "$cf")
}

# build_git <case file> <tree>: real git repositories in the tree, from the
# case's "git" list (ADR-0068). For each: commits in order, each on its branch
# (a new branch starts from main), then tags, then back on main, then files
# stashed, then files staged, then a bare copy and a plain clone, then a
# blob nothing reaches. Files the case writes later are uncommitted changes.
build_git() {
  local cf="$1" tree="$2" n i k dir path branch blob
  local g=(git -c user.name=fixture -c user.email=fixture@localhost -c commit.gpgsign=false -c init.defaultBranch=main)
  n="$(jq -r '.git // [] | length' "$cf")"
  for ((i = 0; i < n; i++)); do
    dir="$tree/$(jq -r --argjson i "$i" '.git[$i].dir' "$cf")"
    mkdir -p "$dir"; "${g[@]}" -C "$dir" init -q
    writes() { # writes <jq path to a file map>
      while IFS= read -r path; do
        mkdir -p "$dir/$(dirname "$path")"
        subst "$(jq -r --argjson i "$i" --arg p "$path" "$1[\$p]" "$cf")" > "$dir/$path"
      done < <(jq -r --argjson i "$i" "$1 // {} | keys[]" "$cf")
    }
    local c commits; commits="$(jq -r --argjson i "$i" '.git[$i].commits // [] | length' "$cf")"
    for ((c = 0; c < commits; c++)); do
      branch="$(jq -r --argjson i "$i" --argjson c "$c" '.git[$i].commits[$c].branch // "main"' "$cf")"
      if [[ "$("${g[@]}" -C "$dir" rev-parse -q --verify HEAD 2>/dev/null)" != "" ]]; then
        if "${g[@]}" -C "$dir" rev-parse -q --verify "refs/heads/$branch" >/dev/null; then
          "${g[@]}" -C "$dir" checkout -q "$branch"
        else
          "${g[@]}" -C "$dir" checkout -q -b "$branch" main
        fi
      fi
      writes ".git[\$i].commits[$c].files"
      while IFS= read -r path; do rm -f "$dir/$path"; done < <(jq -r --argjson i "$i" --argjson c "$c" '.git[$i].commits[$c].remove // [] | .[]' "$cf")
      "${g[@]}" -C "$dir" add -A
      "${g[@]}" -C "$dir" commit -q --allow-empty -m "commit $c"
    done
    while IFS=$'\t' read -r k branch; do
      [[ -n "$k" ]] && "${g[@]}" -C "$dir" tag "$k" "$branch"
    done < <(jq -r --argjson i "$i" '.git[$i].tags // {} | to_entries[] | "\(.key)\t\(.value)"' "$cf")
    "${g[@]}" -C "$dir" rev-parse -q --verify refs/heads/main >/dev/null && "${g[@]}" -C "$dir" checkout -q main
    if [[ "$(jq -r --argjson i "$i" '.git[$i] | has("stash")' "$cf")" == "true" ]]; then
      writes ".git[\$i].stash"
      "${g[@]}" -C "$dir" stash push -q -u
    fi
    if [[ "$(jq -r --argjson i "$i" '.git[$i] | has("stage")' "$cf")" == "true" ]]; then
      writes ".git[\$i].stage"
      "${g[@]}" -C "$dir" add -A
    fi
    k="$(jq -r --argjson i "$i" '.git[$i].bare // empty' "$cf")"
    [[ -n "$k" ]] && "${g[@]}" clone -q --bare "$dir" "$tree/$k"
    k="$(jq -r --argjson i "$i" '.git[$i].clone // empty' "$cf")"
    [[ -n "$k" ]] && "${g[@]}" clone -q --no-checkout "$dir" "$tree/$k" && "${g[@]}" -C "$tree/$k" checkout -q main
    blob="$(jq -r --argjson i "$i" '.git[$i].loose // empty' "$cf")"
    [[ -n "$blob" ]] && subst "$blob" | "${g[@]}" -C "$dir" hash-object -w --stdin >/dev/null
  done
}

# build_forge <case file> <destination>: real git repositories for every
# owner/repository/branch the case describes. Identity and signing are set on
# the command line, so this works on a machine with no git configuration.
build_forge() {
  local cf="$1" dest="$2" owner repo branch path content work
  local bare n i a claimed amend size before head actor
  local g=(git -c user.name=fixture -c user.email=fixture@localhost -c commit.gpgsign=false)
  mkdir -p "$dest/repos" "$dest/git" "$dest/pushes"
  build_files "$cf" forge_files "$dest"
  while IFS= read -r owner; do
    : > "$dest/repos/$owner"
    while IFS= read -r repo; do
      printf '%s/%s\n' "$owner" "$repo" >> "$dest/repos/$owner"
      # Listed and not there: a repository that cannot be copied.
      [[ "$(jq -r --arg o "$owner" --arg r "$repo" '.forge[$o][$r].missing // false' "$cf")" == "true" ]] && continue
      work="$dest/.work/$owner/$repo"; mkdir -p "$work"
      "${g[@]}" -C "$work" init -q -b main
      printf 'base\n' > "$work/BASE.txt"
      "${g[@]}" -C "$work" add -A; "${g[@]}" -C "$work" commit -q -m base
      while IFS= read -r branch; do
        if [[ "$branch" == "main" ]]; then "${g[@]}" -C "$work" checkout -q main
        else "${g[@]}" -C "$work" checkout -q -b "$branch" main; fi
        while IFS= read -r path; do
          content="$(jq -r --arg o "$owner" --arg r "$repo" --arg b "$branch" --arg p "$path" \
                     '.forge[$o][$r].branches[$b][$p]' "$cf")"
          mkdir -p "$work/$(dirname "$path")"
          subst "$content" > "$work/$path"
        done < <(jq -r --arg o "$owner" --arg r "$repo" --arg b "$branch" '.forge[$o][$r].branches[$b] | keys[]' "$cf")
        "${g[@]}" -C "$work" add -A; "${g[@]}" -C "$work" commit -q -m change
      done < <(jq -r --arg o "$owner" --arg r "$repo" '.forge[$o][$r].branches // {} | keys[]' "$cf")
      mkdir -p "$dest/git/$owner" "$dest/pushes/$owner"
      "${g[@]}" -C "$work" checkout -q main
      "${g[@]}" clone -q --bare "$work" "$dest/git/$owner/$repo.git"
      bare="$dest/git/$owner/$repo.git"
      # GitHub serves a commit by its ID whether or not a branch still
      # reaches it. A plain git repository has to be told to.
      "${g[@]}" -C "$bare" config uploadpack.allowAnySHA1InWant true
      content="$(jq -r --arg o "$owner" --arg r "$repo" '.forge[$o][$r].pushes // empty' "$cf")"
      [[ -n "$content" ]] && printf '%s\n' "$content" > "$dest/pushes/$owner/$repo.tsv"
      # Pushes made after the repository existed, each a real push to the
      # pretend GitHub. "force" replaces the newest commit, which leaves the
      # one it replaced reachable from nothing, as a force-push does. With
      # an "actor" the push goes on the record, holding the commit the
      # branch pointed to before it. "date" is what the commit CLAIMS as
      # its date, which is the attacker's to choose.
      n="$(jq -r --arg o "$owner" --arg r "$repo" '.forge[$o][$r].attacks // [] | length' "$cf")"
      for ((i=0; i<n; i++)); do
        a=".forge[\$o][\$r].attacks[$i]"
        branch="$(jq -r --arg o "$owner" --arg r "$repo" "$a.branch" "$cf")"
        "${g[@]}" -C "$work" checkout -q "$branch"
        while IFS= read -r path; do
          content="$(jq -r --arg o "$owner" --arg r "$repo" --arg p "$path" "$a.files[\$p]" "$cf")"
          mkdir -p "$work/$(dirname "$path")"
          subst "$content" > "$work/$path"
        done < <(jq -r --arg o "$owner" --arg r "$repo" "$a.files | keys[]" "$cf")
        "${g[@]}" -C "$work" add -A
        claimed="$(jq -r --arg o "$owner" --arg r "$repo" "$a.date // empty" "$cf")"
        amend=(); size=1
        if [[ "$(jq -r --arg o "$owner" --arg r "$repo" "$a.force // false" "$cf")" == "true" ]]; then
          amend=(--amend); size=0
        fi
        if [[ -n "$claimed" ]]; then
          GIT_AUTHOR_DATE="$claimed" GIT_COMMITTER_DATE="$claimed" \
            "${g[@]}" -C "$work" commit -q ${amend[@]+"${amend[@]}"} -m "update config"
        else
          "${g[@]}" -C "$work" commit -q ${amend[@]+"${amend[@]}"} -m "update config"
        fi
        before="$("${g[@]}" -C "$bare" rev-parse "refs/heads/$branch")"
        "${g[@]}" -C "$work" push -q --force "$bare" "$branch:$branch"
        head="$("${g[@]}" -C "$bare" rev-parse "refs/heads/$branch")"
        actor="$(jq -r --arg o "$owner" --arg r "$repo" "$a.actor // empty" "$cf")"
        [[ -n "$actor" ]] && printf 'refs/heads/%s\t%s\t%s\t%s\t%s\t%s\n' "$branch" "$before" "$head" "$actor" \
          "$(jq -r --arg o "$owner" --arg r "$repo" "$a.at" "$cf")" "$size" >> "$dest/pushes/$owner/$repo.tsv"
      done
    done < <(jq -r --arg o "$owner" '.forge[$o] | keys[]' "$cf")
  done < <(jq -r '.forge // {} | keys[]' "$cf")
  rm -rf "$dest/.work"
}

# forge_refs <forge> <owner/repo>: where every branch and tag of one
# repository points, and what was set on it. Empty for one that is not there.
forge_refs() {
  git -C "$1/git/$2.git" for-each-ref --format='%(refname) %(objectname)' 2>/dev/null || true
  ls "$1/changed/$2".* 2>/dev/null || true
}

# forge_repos <case file>: owner/repo for every repository of the case.
forge_repos() { jq -r '.forge // {} | to_entries[] | .key as $o | .value | keys[] | "\($o)/\(.)"' "$1"; }

PASS=0; FAIL=0; FAILED_CASES=()
FORGE_STATE=""
HOST_STATE=""
COMMAND="check"
CASE_FILE=""
TREE=""

run_case() {
  local cf="$1" name; name="$(basename "$cf" .json)"
  [[ -n "$ONLY" && "$ONLY" != "$name" ]] && return 0

  local why; why="$(jq -r '.why' "$cf")"

  local is_host; is_host="$(jq -r 'has("host")' "$cf")"
  COMMAND="$(jq -r '.command // "check"' "$cf")"

  local tmp; tmp="$(mktemp -d)"
  FAKE_HOME="$tmp/home"; mkdir -p "$FAKE_HOME" "$tmp/tree" "$tmp/out"

  CASE_FILE="$cf"; TREE="$tmp/tree"
  KILL_SIGNAL="$(jq -r '.kill.signal // empty' "$cf")"
  KILL_WHEN="$(jq -r '.kill.when // empty' "$cf")"
  KILL_IN="$(jq -r '.kill.in // "report"' "$cf")"
  EXTRA_ARGS=()
  HOME_FLAG="$(jq -r 'if has("home_flag") then .home_flag else true end' "$cf")"
  NO_GIT="$(jq -r '.no_git // false' "$cf")"
  # Flags of the case's own for its first run, as then.args are for a second.
  while IFS= read -r ed; do EXTRA_ARGS+=("$ed"); done < <(jq -r '.args[]? // empty' "$cf")

  # build the fixture: the tree to scan, the home directory, and for a host
  # case the machine's state
  # Repositories first: files the case writes after them are changes not yet
  # committed, and hooks go into a .git that exists.
  build_git "$cf" "$tmp/tree"
  build_files "$cf" files "$tmp/tree"
  build_files_exact "$cf" files_exact "$tmp/tree"
  build_files "$cf" home "$FAKE_HOME"
  # Files that must be executable, such as git hooks.
  local x
  while IFS= read -r x; do chmod +x "$tmp/tree/$x"; done < <(jq -r '.executable[]? // empty' "$cf")
  # Files too large to write out in a case: their first bytes, then zeros to
  # the size given in MB. For the large-file hash sweep (ADR-0059).
  # With "fill", the rest is that character instead: a large text file,
  # which takes time to read.
  local big_path big_head big_mb big_fill
  while IFS=$'\t' read -r big_path big_head big_mb big_fill; do
    [[ -z "$big_path" ]] && continue
    mkdir -p "$tmp/tree/$(dirname "$big_path")"
    # The placeholders work in the first bytes too: a binary can carry a
    # weak string after its first NUL byte.
    printf '%b' "$(subst "$big_head")" > "$tmp/tree/$big_path"
    if [[ -n "$big_fill" ]]; then
      head -c $((big_mb * 1048576)) /dev/zero | tr '\0' "$big_fill" >> "$tmp/tree/$big_path"
    else
      dd if=/dev/zero bs=1048576 count="$big_mb" 2>/dev/null >> "$tmp/tree/$big_path"
    fi
  done < <(jq -r '.large_files // [] | .[] | "\(.path)\t\(.head)\t\(.mb)\t\(.fill // "")"' "$cf")
  # Many small files with the same content, named 00000.js upwards: a tree
  # with save points in it (ADR-0060). A file the case names is kept.
  local many_dir many_count many_content many_i many_f
  while IFS=$'\t' read -r many_dir many_count many_content; do
    [[ -z "$many_dir" ]] && continue
    mkdir -p "$tmp/tree/$many_dir"
    for ((many_i = 0; many_i < many_count; many_i++)); do
      printf -v many_f '%s/%05d.js' "$tmp/tree/$many_dir" "$many_i"
      [[ -e "$many_f" ]] || printf '%s\n' "$many_content" > "$many_f"
    done
  done < <(jq -r '.many_files // [] | .[] | "\(.dir)\t\(.count)\t\(.content)"' "$cf")
  FORGE_STATE=""
  if [[ "$(jq -r 'has("forge") or has("forge_files")' "$cf")" == "true" ]]; then
    FORGE_STATE="$tmp/forge"
    build_forge "$cf" "$FORGE_STATE"
  fi
  IOC_DIR=""
  if [[ "$(jq -r 'has("known_hashes")' "$cf")" == "true" ]]; then
    IOC_DIR="$tmp/ioc"
    cp -R "$ROOT/ioc" "$IOC_DIR"
    local known_path known_label known_sum
    while IFS=$'\t' read -r known_path known_label; do
      known_sum="$( { sha256sum "$tmp/tree/$known_path" 2>/dev/null \
        || shasum -a 256 "$tmp/tree/$known_path"; } | cut -d' ' -f1)"
      printf '%s  %s\n' "$known_sum" "$known_label" >> "$IOC_DIR/hashes.txt"
    done < <(jq -r '.known_hashes[] | [.path, .label] | @tsv' "$cf")
  fi
  HOST_STATE=""
  if [[ "$is_host" == "true" ]]; then
    HOST_STATE="$tmp/host"; mkdir -p "$HOST_STATE"
    build_files "$cf" host "$HOST_STATE"
  fi

  # roots, relative to the fixture tree
  local roots=(); local r
  while IFS= read -r r; do roots+=("$tmp/tree/$r"); done < <(jq -r '.roots[]' "$cf")

  local apply qdir=""
  apply="$(jq -r '.apply // false' "$cf")"
  [[ "$apply" == "true" ]] && qdir="$tmp/out/quarantine"

  # For a host case the home directory and the supplied state are part of what
  # a read-only run must leave alone: that is where persistence lives.
  local before after out rc nwo
  if [[ -n "$FORGE_STATE" ]]; then
    mkdir -p "$tmp/forge-before"
    while IFS= read -r nwo; do
      forge_refs "$FORGE_STATE" "$nwo" > "$tmp/forge-before/${nwo//\//__}"
    done < <(forge_repos "$cf")
  fi
  before="$(snapshot "$tmp/tree")"
  [[ -n "$HOST_STATE" ]] && before+="$(snapshot "$FAKE_HOME")$(snapshot "$HOST_STATE")"
  # Where the report goes: the usual place, a path inside the tree being
  # scanned, or nowhere given, so the run picks its own in the home directory.
  local rep="$tmp/out/report.txt" rep_arg
  if [[ "$(jq -r '.report_in_tree // empty' "$cf")" != "" ]]; then
    rep="$tmp/tree/$(jq -r '.report_in_tree' "$cf")"
  fi
  rep_arg="$rep"
  [[ "$(jq -r '.no_report // false' "$cf")" == "true" ]] && rep_arg=""
  out="$("impl_$IMPL" "$rep_arg" "$tmp/out/state" "$qdir" "${roots[@]}")"
  rc=$?
  # A case of two runs: the first, killed part way, leaves a progress file;
  # what is between them is done to it or to its report; the second run is
  # the one the expectations are about (ADR-0060).
  local first_command="$COMMAND"
  if [[ "$(jq -r 'has("then")' "$cf")" == "true" ]]; then
    local prog="${rep%.*}.progress" ed
    while IFS= read -r ed; do
      [[ -z "$ed" ]] && continue
      sed -e "$ed" "$prog" > "$prog.edit" && mv "$prog.edit" "$prog"
    done < <(jq -r '.between.progress_sed[]? // empty' "$cf")
    while IFS= read -r ed; do
      [[ -z "$ed" ]] && continue
      sed -e "$ed" "$rep" > "$rep.edit" && mv "$rep.edit" "$rep"
    done < <(jq -r '.between.report_sed[]? // empty' "$cf")
    [[ "$(jq -r '.between.remove_report // false' "$cf")" == "true" ]] && rm -f "$rep"
    KILL_SIGNAL=""
    COMMAND="$(jq -r --arg c "$first_command" '.then.command // $c' "$cf")"
    EXTRA_ARGS=()
    while IFS= read -r ed; do EXTRA_ARGS+=("$ed"); done < <(jq -r '.then.args[]? // empty' "$cf")
    local then_roots=()
    if [[ "$(jq -r '.then | has("roots")' "$cf")" == "true" ]]; then
      while IFS= read -r r; do then_roots+=("$tmp/tree/$r"); done < <(jq -r '.then.roots[]' "$cf")
    else
      then_roots=("${roots[@]}")
    fi
    out="$("impl_$IMPL" "$rep_arg" "$tmp/out/state" "$qdir" "${then_roots[@]}")"
    rc=$?
    COMMAND="$first_command"
  fi
  # The same tree checked once, from the start and never stopped: what a
  # continued check must agree with, exit code and every finding.
  local whole="" whole_rc=""
  if [[ "$(jq -r '.compare_uninterrupted // false' "$cf")" == "true" ]]; then
    mkdir -p "$tmp/whole"
    KILL_SIGNAL=""; EXTRA_ARGS=()
    whole="$("impl_$IMPL" "$tmp/whole/report.txt" "$tmp/whole/state" "" "${roots[@]}")"
    whole_rc=$?
  fi
  if [[ -z "$rep_arg" ]]; then
    rep="$(find "$FAKE_HOME" -maxdepth 1 -name 'polinrider-report-*.txt' 2>/dev/null | sed -n 1p)"
  fi
  after="$(snapshot "$tmp/tree")"
  [[ -n "$HOST_STATE" ]] && after+="$(snapshot "$FAKE_HOME")$(snapshot "$HOST_STATE")"

  # --- assertions ----------------------------------------------------------
  local errs=()

  local want_exit; want_exit="$(jq -r '.expect.exit' "$cf")"
  [[ "$rc" == "$want_exit" ]] || errs+=("exit code $rc, expected $want_exit")

  # findings that must be present: level + substring, and the path if given
  local n i level match fpath line
  n="$(jq -r '.expect.findings | length' "$cf")"
  for ((i=0; i<n; i++)); do
    level="$(jq -r ".expect.findings[$i].level" "$cf")"
    match="$(jq -r ".expect.findings[$i].match" "$cf")"
    fpath="$(jq -r ".expect.findings[$i].path // empty" "$cf")"
    line="$(printf '%s\n' "$out" | grep -F "[$level]" | grep -F "$match" || true)"
    if [[ -z "$line" ]]; then
      errs+=("missing [$level] finding matching: $match")
    elif [[ -n "$fpath" ]] && ! grep -qF "$fpath" <<<"$line"; then
      errs+=("[$level] '$match' did not name the expected path: $fpath")
    fi
  done

  # strings that must never appear. This is the false-positive and the
  # silently-skipped-work guard, and it is the more important half.
  local findings_only
  findings_only="$(printf '%s\n' "$out" | grep -E '\[(HIT|review)\]' || true)"
  while IFS= read -r forbidden; do
    [[ -z "$forbidden" ]] && continue
    grep -qF "$forbidden" <<<"$findings_only" \
      && errs+=("reported as a finding, and must not be: $forbidden")
  done < <(jq -r '.expect.must_not_report[]? // empty' "$cf")

  # strings that must appear nowhere at all, on the console or in the report
  # file. This is the redaction guard: a secret the scan read must not be a
  # secret the scan wrote down.
  local everything="$out"
  [[ -n "$rep" && -f "$rep" ]] && everything+="$(cat "$rep")"
  while IFS= read -r forbidden; do
    [[ -z "$forbidden" ]] && continue
    grep -qF "$forbidden" <<<"$everything" \
      && errs+=("printed, and must never be: $forbidden")
  done < <(jq -r '.expect.must_not_print[]? // empty' "$cf")

  # strings that must appear somewhere in the output. For advice printed under
  # a finding, which carries no level tag of its own to match on. Here-strings,
  # not a pipe: with pipefail on, a grep -q that stops at its first match can
  # break printf's pipe, and found would read as not found.
  while IFS= read -r wanted; do
    [[ -z "$wanted" ]] && continue
    grep -qF "$wanted" <<<"$out" \
      || errs+=("not printed, and must be: $wanted")
  done < <(jq -r '.expect.must_print[]? // empty' "$cf")

  # strings that must appear in the console output in this order: a step and
  # what it found before the next step starts (ADR-0059).
  local after_line=0 at
  while IFS= read -r wanted; do
    [[ -z "$wanted" ]] && continue
    at="$(printf '%s\n' "$out" | awk -v from="$after_line" -v w="$wanted" \
      'NR > from && index($0, w) { print NR; exit }')"
    if [[ -z "$at" ]]; then
      errs+=("not printed in order, after line $after_line: $wanted")
      break
    fi
    after_line="$at"
  done < <(jq -r '.expect.print_order[]? // empty' "$cf")

  # strings the report file must hold, as written while the scan ran
  while IFS= read -r wanted; do
    [[ -z "$wanted" ]] && continue
    if [[ -z "$rep" || ! -f "$rep" ]]; then
      errs+=("no report file was written")
      break
    fi
    grep -qF "$wanted" "$rep" || errs+=("not in the report file, and must be: $wanted")
  done < <(jq -r '.expect.report_must_contain[]? // empty' "$cf")

  # strings the report file must not hold once the run has ended
  while IFS= read -r forbidden; do
    [[ -z "$forbidden" ]] && continue
    if [[ -z "$rep" || ! -f "$rep" ]]; then
      errs+=("no report file was written")
      break
    fi
    grep -qF "$forbidden" "$rep" && errs+=("in the report file, and must not be: $forbidden")
  done < <(jq -r '.expect.report_must_not_contain[]? // empty' "$cf")

  # whether the progress file is there once the run has ended
  local want_progress
  want_progress="$(jq -r '.expect.progress_after // empty' "$cf")"
  if [[ -n "$want_progress" ]]; then
    local have_progress=absent
    [[ -f "${rep%.*}.progress" ]] && have_progress=present
    [[ "$have_progress" == "$want_progress" ]] \
      || errs+=("the progress file is $have_progress afterwards, expected $want_progress")
  fi

  if [[ -n "$whole_rc" ]]; then
    [[ "$rc" == "$whole_rc" ]] \
      || errs+=("exit code $rc, and $whole_rc for the same tree checked from the start")
    local found_here found_whole
    found_here="$(printf '%s\n' "$out" | grep -E '\[(HIT|review)\]' | LC_ALL=C sort || true)"
    found_whole="$(printf '%s\n' "$whole" | grep -E '\[(HIT|review)\]' | LC_ALL=C sort || true)"
    [[ "$found_here" == "$found_whole" ]] \
      || errs+=("findings differ from the same tree checked from the start")
  fi

  # the report file's first line, exactly
  local want_first
  want_first="$(jq -r '.expect.report_first_line // empty' "$cf")"
  if [[ -n "$want_first" ]]; then
    if [[ -z "$rep" || ! -f "$rep" ]]; then
      errs+=("no report file was written")
    elif [[ "$(sed -n 1p "$rep")" != "$want_first" ]]; then
      errs+=("the report's first line is '$(sed -n 1p "$rep")', expected: $want_first")
    fi
  fi

  # files that must hold exactly this afterwards. Compared without trailing
  # newlines, which the fixture builder does not control on every platform.
  local want_path want_body
  while IFS= read -r want_path; do
    [[ -z "$want_path" ]] && continue
    want_body="$(subst "$(jq -r --arg p "$want_path" '.expect.file_after[$p]' "$cf")")"
    if [[ ! -f "$tmp/tree/$want_path" ]]; then
      errs+=("missing afterwards: $want_path")
    elif [[ "$(cat "$tmp/tree/$want_path")" != "$want_body" ]]; then
      errs+=("not what it should be afterwards: $want_path")
    fi
  done < <(jq -r '.expect.file_after // {} | keys[]' "$cf")

  # the scanned tree must be untouched unless the case says otherwise
  local may_change; may_change="$(jq -r '.expect.tree_may_change // false' "$cf")"
  if [[ "$may_change" != "true" && "$before" != "$after" ]]; then
    errs+=("the scanned tree changed during a read-only run")
  fi

  # The pretend GitHub. A repository the case does not name under
  # "forge_after" must be exactly as it was: every branch, every tag, its
  # description and whether it is archived. This is the guard on the half of
  # the tool that pushes, and it is on by default.
  if [[ -n "$FORGE_STATE" ]]; then
    local fa bare want got
    while IFS= read -r nwo; do
      fa="$(jq -c --arg n "$nwo" '.expect.forge_after[$n] // empty' "$cf")"
      bare="$FORGE_STATE/git/$nwo.git"
      if [[ -z "$fa" || "$(jq -r '.unchanged // false' <<<"$fa")" == "true" ]]; then
        [[ "$(forge_refs "$FORGE_STATE" "$nwo")" == "$(cat "$tmp/forge-before/${nwo//\//__}")" ]] \
          || errs+=("$nwo was changed on the pretend GitHub, and must not be")
        continue
      fi
      while IFS= read -r want; do
        git -C "$bare" grep -qF "$STRONG" "refs/heads/$want" 2>/dev/null \
          && errs+=("$nwo: $want still carries the indicator")
      done < <(jq -r '.clean[]? // empty' <<<"$fa")
      while IFS= read -r want; do
        git -C "$bare" grep -qF "$STRONG" "refs/heads/$want" 2>/dev/null \
          || errs+=("$nwo: $want no longer carries the indicator, and the case says it stays")
      done < <(jq -r '.infected[]? // empty' <<<"$fa")
      # Read whole and then searched: with pipefail on, a grep -q that stops
      # at its first match fails the pipe, and found would read as not found.
      got="$(git -C "$bare" log --branches --tags -p --format=%H 2>/dev/null)"
      case "$(jq -r '.history // empty' <<<"$fa")" in
        clean)    grep -qF "$STRONG" <<<"$got" \
                    && errs+=("$nwo: the indicator is still in the history") ;;
        infected) grep -qF "$STRONG" <<<"$got" \
                    || errs+=("$nwo: the history was rewritten, and the case says it is not") ;;
      esac
      while IFS= read -r want; do
        got="$(git -C "$bare" cat-file blob "$want" 2>/dev/null)" \
          || { errs+=("$nwo: missing afterwards: $want"); continue; }
        [[ "$got" == "$(subst "$(jq -r --arg k "$want" '.file[$k]' <<<"$fa")")" ]] \
          || errs+=("$nwo: not what it should be afterwards: $want")
      done < <(jq -r '.file // {} | keys[]' <<<"$fa")
      want="$(jq -r '.readme_top // empty' <<<"$fa")"
      if [[ -n "$want" ]]; then
        got="$(git -C "$bare" cat-file blob HEAD:README.md 2>/dev/null | sed -n 1p)"
        [[ "$got" == "$want" ]] || errs+=("$nwo: the README does not open with the notice")
      fi
      want="$(jq -r '.readme_keeps // empty' <<<"$fa")"
      if [[ -n "$want" ]]; then
        got="$(git -C "$bare" cat-file blob HEAD:README.md 2>/dev/null)"
        grep -qF "$want" <<<"$got" \
          || errs+=("$nwo: the README lost what it held before: $want")
      fi
      if [[ "$(jq -r '.archived // false' <<<"$fa")" == "true" ]]; then
        [[ -f "$FORGE_STATE/changed/$nwo.archived" ]] || errs+=("$nwo: was not archived")
        grep -qF "INFECTED" "$FORGE_STATE/changed/$nwo.description" 2>/dev/null \
          || errs+=("$nwo: the description was not replaced")
      else
        [[ -e "$FORGE_STATE/changed/$nwo.archived" ]] && errs+=("$nwo: was archived, and must not be")
      fi
    done < <(forge_repos "$cf")
  fi

  # --apply moves into quarantine and never deletes
  if [[ "$apply" == "true" ]]; then
    while IFS= read -r q; do
      [[ -z "$q" ]] && continue
      [[ -e "$tmp/tree/$q" ]] && errs+=("still in the tree after --apply: $q")
      find "$qdir" -type f -path "*${q##*/}" 2>/dev/null | grep -q . \
        || errs+=("not found in quarantine after --apply: $q")
    done < <(jq -r '.expect.quarantined[]? // empty' "$cf")
    # a stripped file stays where it was, without the indicator, and the
    # original, with it, is in quarantine
    while IFS= read -r q; do
      [[ -z "$q" ]] && continue
      [[ -f "$tmp/tree/$q" ]] || { errs+=("stripped file is gone from the tree: $q"); continue; }
      grep -qF "$STRONG" "$tmp/tree/$q" && errs+=("still carries the indicator after the strip: $q")
      local kept; kept="$(find "$qdir" -type f -path "*${q##*/}" 2>/dev/null | sed -n 1p)"
      if [[ -z "$kept" ]]; then errs+=("original not kept in quarantine: $q")
      elif ! grep -qF "$STRONG" "$kept"; then errs+=("the copy in quarantine is not the infected original: $q"); fi
    done < <(jq -r '.expect.stripped[]? // empty' "$cf")
    # the same, for an artifact that lived in the home directory
    while IFS= read -r q; do
      [[ -z "$q" ]] && continue
      [[ -e "$FAKE_HOME/$q" ]] && errs+=("still in the home directory after --apply: $q")
      find "$qdir" -type f -path "*${q##*/}" 2>/dev/null | grep -q . \
        || errs+=("not found in quarantine after --apply: $q")
    done < <(jq -r '.expect.home_quarantined[]? // empty' "$cf")
    [[ -f "$qdir/manifest.tsv" ]] || errs+=("--apply wrote no quarantine manifest")
  fi

  if [[ ${#errs[@]} -eq 0 ]]; then
    PASS=$((PASS+1)); printf '  \033[32mpass\033[0m  %-38s %s\n' "$name" "$why"
    [[ $SHOW_DIFF -eq 1 ]] && printf '%s\n' "$out" | sed 's/^/        /'
  else
    FAIL=$((FAIL+1)); FAILED_CASES+=("$name")
    printf '  \033[31mFAIL\033[0m  %-38s %s\n' "$name" "$why"
    printf '        %s\n' "${errs[@]}"
    if [[ -n "$ONLY" || $SHOW_DIFF -eq 1 ]]; then
      printf '        --- run output ---\n'; printf '%s\n' "$out" | sed 's/^/        /'
    fi
  fi
  rm -rf "$tmp"
}

# --- refusals -------------------------------------------------------------
# The shell accepted some of these by ignoring them. That is the behaviour
# the binary replaced, and why each is pinned.
refusals() {
  local bin="${CARGO_TARGET_DIR:-$ROOT/target}/release/polinrider"
  [[ -x "$bin" ]] || return 0
  local tmp; tmp="$(mktemp -d)"

  check_refusal() {
    local why="$1"; shift
    local out rc
    out="$("$bin" "$@" 2>&1)"; rc=$?
    if [[ $rc -eq 3 ]]; then
      PASS=$((PASS+1)); printf '  \033[32mpass\033[0m  %-38s %s\n' "refuses" "$why"
    else
      FAIL=$((FAIL+1)); FAILED_CASES+=("refuse:$why")
      printf '  \033[31mFAIL\033[0m  %-38s %s\n' "refuses" "$why"
      printf '        exit %s, expected 3\n        %s\n' "$rc" "${out%%$'\n'*}"
    fi
  }

  check_refusal "an unknown flag"            check --not-a-real-flag "$tmp"
  check_refusal "a flag it has not built"    check --jobs 4 "$tmp"
  check_refusal "a flag 2.0 removed, --state"        check --state /tmp/s "$tmp"
  # --resume is real again, for a check that moves nothing (ADR-0060).
  check_refusal "clean told to --resume"             clean --resume "$tmp"
  check_refusal "the guided flow told to --resume"   guide --resume
  check_refusal "--resume alongside --fresh"         check --resume --fresh "$tmp"
  check_refusal "--resume alongside --apply"         check --apply --resume "$tmp"
  check_refusal "clean told to --git-all"            clean --git-all "$tmp"
  check_refusal "--no-git alongside --git-all"       check --no-git --git-all "$tmp"
  check_refusal "the guided flow told to --no-git"   guide --no-git
  check_refusal "a root that does not exist" check "$tmp/definitely-absent"
  check_refusal "no root at all"             check --fs-only
  check_refusal "a missing indicator set"    check --ioc "$tmp/no-ioc-here" "$tmp"
  check_refusal "a root that is a file"      check "$0"
  check_refusal "host state that is not there"       check --host-state "$tmp/no-state-here" "$tmp"
  check_refusal "host state alongside --fs-only"     check --fs-only --host-state "$tmp" "$tmp"
  check_refusal "host state that names no platform"  check --host-state "$tmp" "$tmp"
  check_refusal "the guided flow told to --apply"    guide --apply
  check_refusal "the guided flow handed a directory" guide "$tmp"
  check_refusal "clean with no directory"            clean
  # A content pattern with no fixed part could make a long line slow, so it
  # stops the scan rather than run (ADR-0052).
  cp -R "$ROOT/ioc" "$tmp/badioc" && printf '[0-9]+\n' >> "$tmp/badioc/strong-patterns.txt"
  check_refusal "a content pattern with no fixed part" check --fs-only --ioc "$tmp/badioc" "$tmp"
  check_refusal "--purge-evidence handed a directory" --purge-evidence "$tmp"
  check_refusal "--purge-evidence told to --apply"   --purge-evidence --apply

  # --purge-evidence, the one delete (ADR-0046). A folder of polinrider's own
  # copies goes on a typed yes and on nothing else. A folder holding anything
  # else is refused whole, and input that ends deletes nothing.
  check_purge() {
    local why="$1" want="$2" answer="$3" stray="$4" kept="$5"
    local dir="$tmp/evidence-$PASS-$FAIL" out rc
    rm -rf "$dir"; mkdir -p "$dir"
    git init -q --bare "$dir/acme__shop.git"
    : > "$dir/indicators-strong.txt"; : > "$dir/indicators-packages.txt"; : > "$dir/indicators-weak.txt"
    [[ -n "$stray" ]] && printf 'mine\n' > "$dir/$stray"
    if [[ "$answer" == "<eof>" ]]; then
      out="$("$bin" --purge-evidence --evidence "$dir" 2>&1 </dev/null)"; rc=$?
    else
      out="$(printf '%s\n' "$answer" | "$bin" --purge-evidence --evidence "$dir" 2>&1)"; rc=$?
    fi
    local there=no; [[ -d "$dir" ]] && there=yes
    if [[ $rc -eq $want && "$there" == "$kept" ]]; then
      PASS=$((PASS+1)); printf '  \033[32mpass\033[0m  %-38s %s\n' "purge" "$why"
    else
      FAIL=$((FAIL+1)); FAILED_CASES+=("purge:$why")
      printf '  \033[31mFAIL\033[0m  %-38s %s\n' "purge" "$why"
      printf '        exit %s (expected %s), folder still there: %s (expected %s)\n' "$rc" "$want" "$there" "$kept"
      printf '%s\n' "$out" | sed 's/^/        /'
    fi
    rm -rf "$dir"
  }
  check_purge "its own copies go on yes"                 0 yes   ""          no
  check_purge "no leaves them"                           0 no    ""          yes
  check_purge "Enter is not yes, and input then ends"    3 ""    ""          yes
  check_purge "input that ends deletes nothing"          3 "<eof>" ""        yes
  check_purge "a folder holding anything else is refused" 3 yes  "notes.txt" yes

  # A reader that goes away. `polinrider | head` is an ordinary thing to type,
  # and Rust's println! panics on a closed pipe. The run must end without a
  # word on stderr and with the exit code it would have had anyway. The
  # reader here closes its end at once and stays alive, so the write that
  # fails is the binary's and not a race the binary usually wins.
  check_closed_pipe() {
    local why="$1" want="$2"; shift 2
    local err rc
    err="$( { "$bin" "$@" | { exec 0<&-; sleep 1; }; exit "${PIPESTATUS[0]}"; } 2>&1 >/dev/null )"; rc=$?
    if [[ -z "$err" && $rc -eq $want ]]; then
      PASS=$((PASS+1)); printf '  \033[32mpass\033[0m  %-38s %s\n' "closed pipe" "$why"
    else
      FAIL=$((FAIL+1)); FAILED_CASES+=("pipe:$why")
      printf '  \033[31mFAIL\033[0m  %-38s %s\n' "closed pipe" "$why"
      printf '        exit %s, expected %s\n        %s\n' "$rc" "$want" "${err%%$'\n'*}"
    fi
  }
  mkdir -p "$tmp/empty"
  check_closed_pipe "--version, nobody reading"       0 --version
  check_closed_pipe "a clean scan, nobody reading"    0 check --fs-only "$tmp/empty"
  check_refusal "clean handed host state"            clean --host-state "$tmp" "$tmp"
  check_refusal "clean handed --fs-only"             clean --fs-only "$tmp"
  rm -rf "$tmp"
}

printf 'conformance: indicators from ioc/\n\n'
for cf in "$HERE"/cases/*.json; do run_case "$cf"; done
[[ -z "$ONLY" ]] && refusals

printf '\n  %d passed, %d failed\n' "$PASS" "$FAIL"
if [[ $FAIL -gt 0 ]]; then
  printf '  failed: %s\n' "${FAILED_CASES[*]}"
  printf '  one case in detail:  ./conformance/run.sh --case %s\n' "${FAILED_CASES[0]}"
  exit 1
fi
[[ $PASS -eq 0 ]] && { echo "  no cases ran"; exit 3; }
exit 0
