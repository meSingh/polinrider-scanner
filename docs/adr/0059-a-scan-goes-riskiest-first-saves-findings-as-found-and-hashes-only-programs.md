# 0059. A scan goes riskiest first, saves each finding as found and hashes only programs

| | |
|---|---|
| **Status** | Accepted |
| **Date** | 2026-11-15 |

## Context

Mandeep ran `check` on a backup drive of about 700 GB from his old infected
system, on a test machine. It ran for 24 hours, the battery died and every
result was lost. ADR-0030 named exactly this as the reason to revisit its
decision not to checkpoint, and suggested the honest fix might be to bound
the hash sweep rather than bring checkpoints back.

In 3.0.1 a scan was, in order: a full walk, which said nothing; the implant
check, which hashed every file of 10 MB to 300 MB in full (videos, disk
images and archives, on a backup drive); and only then the folder-open tasks,
build configs, git hooks and host checks. Findings were held in memory and
printed, and the report written, only at the end.

Measured in the sandbox on a synthetic tree of 289,500 small files in 1,500
projects plus 150 large non-program files (6.3 GB) and two 15 MB ELF files,
warm cache, with an infected `tasks.json` and build config planted:

| | 3.0.1 | This change |
|---|---|---|
| Whole check | 49 to 54 s | 10.2 s |
| The planted `tasks.json` shown | at the end, about 50 s | 0.17 s |
| Hash sweep | about 38 s, 152 files read in full | 0.4 s, 2 files read |

The hash sweep was most of the time, as was inferred for the real drive. That
drive has not been measured.

## Decision

ADR-0030 stands: no checkpoint, no resume. A run that stops starts again from
the beginning. What changes is what a run does first, what it keeps if it
dies and how much it reads.

1. **Three steps, riskiest first.**
   - Step 1, "Things that run by themselves": the host checks (implant
     install paths and processes, editor extensions, SSH keys, persistence,
     shell startup files, npm's configuration and program files,
     interpreters, connections), then every `.vscode/tasks.json`, build config
     and git hook, then Python startup files.
   - Step 2, "Project files": fonts, the propagation script, named files,
     packages, then one pass that reads each file once, every JavaScript and
     TypeScript file for a confirmed indicator (ADR-0058) and every file up to
     1 MiB for the weak signals (ADR-0047), then credentials.
   - Step 3, "Large files against known hashes".

   **How step 1 avoids waiting for the walk.** The host checks need no walk
   and run first. The walk then shows each file and `.git` directory to the
   scan the moment it lists it, and a folder-open task, a build config or a
   repository's hooks are read there and then. A targeted first pass was
   considered and refused: there is no way to know where the projects on a
   drive are without listing it, so a "quick" pass is either the same walk or
   a guess that misses what it did not guess. Listing reads directory
   entries only, which is cheap (289,500 files in 0.4 s here); the cost was
   always the reading. So step 1 ends when the listing ends, and every step 1
   finding is on the screen and in the report as soon as the walk reaches it.

2. **Findings are streamed.** Each confirmed and review finding is printed
   the moment it is made and appended to the report file, which is synced to
   the disk line by line: a laptop whose battery dies loses its write cache
   too. The full report (every section, the result and the next steps) is
   appended at the end, as before. Until it is, the report's first line says
   the scan is in progress ("PolinRider local check, in progress. The full
   report replaces this when the scan ends."); at the end the finished
   report, the same with its plain title, is written beside it and moved over
   it, so the file is always one or the other. `check` and `clean` with no
   `--report` now save one in the home directory, as the guided flow does,
   and say where before the first step. The guided flow streams the same
   lines into its report while a scan runs; the full report still replaces it
   when the session ends. This run's report is never listed by the walk, so
   it cannot become a weak signal of its own making.

3. **The hash sweep reads only programs.** Every file of 10 MB to 300 MB is
   still considered, but only one whose first bytes are an ELF, Mach-O (thin
   in either byte order, or fat) or Windows PE (`MZ`) header is hashed. Equal
   hashes mean equal bytes, so equal headers. The sweep exists for the
   MicrosoftSystem64 binaries, and every one of them is a program:

   | Hash in `hashes.txt` | Format, from the source | Size |
   |---|---|---|
   | `f2c754a7…` Windows | PE32+ x64 (JFrog) | about 67 MB |
   | `457a3323…` macOS x64 | Mach-O x64 (JFrog) | about 87 MB |
   | `2fec04f2…` macOS arm64 | Mach-O arm64 (JFrog) | about 84 MB |
   | `b505f1d1…` Linux | ELF x64 (JFrog) | about 85 MB |
   | `b2954c94…` Linux build 1.0.8 | ELF 64-bit LSB executable, x86-64 (SafeDep, `file` output) | 85,134,080 bytes |

   Sources: JFrog Security Research, "js-logger-pack Operator Turns Hugging
   Face into a Malware CDN and Exfiltration Backend", 23 April 2026; SafeDep,
   "Inside MicrosoftSystem64", 28 May 2026. JFrog states the formats; the
   sizes of the first four are SafeDep's description of the listing they
   were served from, not tied to the hashes. The other entries in
   `hashes.txt` (the dropper, `tasks.json`, the fake font, the `ulid-xyz`
   files) are far under 10 MB and are matched where the checks already read
   them, at any size (ADR-0045), which is unchanged.

4. **No signal is caught.** Ctrl-C, a closed terminal (SIGHUP) or a kill
   (SIGTERM) ends the process where it is. The report keeps every finding
   made before it, by decision 2, and its first line still says the scan was
   in progress, so it can never read as a clean result. There is no stop
   message, and the exit code is the signal's (130 or 143 from a shell), not
   0. Catching a signal needs either a few lines of `unsafe` code calling the
   system's `signal` (and on Windows `SetConsoleCtrlHandler`) or a first
   dependency such as `signal-hook`, and the crate forbids unsafe code and
   has no dependencies, by choice. A stop path built for the handler, with
   its own message and exit 3, was removed rather than left with nothing to
   reach it. Mandeep's decision, 2026-11-15.

5. **The machine is kept awake** while a scan runs: on macOS by
   `/usr/bin/caffeinate -i -w <pid>`; on Linux by `systemd-inhibit` holding an
   idle and sleep lock while the process lives, where systemd is there. Both
   are run by a fixed path with fixed arguments, and end with the scan, or by
   themselves if it is killed. A helper that is missing is passed over.

6. **The time each step took** is in the report.

## Consequences

- **A day-long run that dies keeps what it found.** The report holds every
  finding made before the end, synced as it was made, under a first line
  that says it is not a result.
- **A stopped run says nothing.** Whoever presses Ctrl-C sees the shell
  prompt and no line saying the result is incomplete; the report says it.
  The corpus pins the report (`a-report-left-by-a-killed-scan-says-it-is-not-a-result`).
- **The answer that matters most comes first.** On the synthetic tree the
  planted task was on screen in 0.17 s instead of at the end.
- **The sweep is bounded by what can match.** On a drive of media and
  archives it reads four bytes of each large file instead of all of it.
- **A large known file that is not a program would now be missed.** A hash
  of, say, a 20 MB archive could be added to `hashes.txt` and never be looked
  for. The corpus pins this (`the-hash-sweep-reads-only-files-that-start-like-a-program`),
  and an entry of that size must be a program, or the rule changed with it.
- **Java class files share the fat Mach-O magic** and are hashed. That costs
  time and loses nothing.
- **The report's sections are in a new order**, and the implant section is
  split in two: the install paths and processes in step 1 ("no second-stage
  implant at its install paths, and none running"), the hash sweep in step 3
  ("Large files against known hashes"). Its old line "no second-stage
  implant found" claimed the sweep too, which step 1 cannot.
- **`check` now writes a file by default**, in the home directory. A run that
  is read-only for the scanned paths still writes its report.
- **A step that is waited on is not shown as waited on.** Step 1 includes
  the walk, so on a slow drive step 1 runs until the listing is done, while
  its findings are already shown.
- **The machine is not kept awake on Windows**, for the same reason no
  signal is caught. Asking needs a system call (`SetThreadExecutionState`)
  that this crate does not make, since it forbids unsafe code, and a helper
  process would need PowerShell. A Windows scan of a large drive should be
  run with sleep turned off by hand.
- **Not measured on the real drive.** The tree here is synthetic and its
  cache warm; a cold USB drive will list more slowly and read more slowly.

## Related

- ADR-0030: no checkpoint. Amended here: the order, the streaming and the
  bounded sweep are what replace it.
- ADR-0025: the pruned walk, now one that shows each file as it is listed.
- ADR-0045: the small files hashed at any size, unchanged.
- ADR-0047 and ADR-0058: the weak signals and the JavaScript files, now
  read in one pass.
