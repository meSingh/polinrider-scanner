//! The history git keeps in each repository the walk found: the last commit,
//! the staged files, every branch, remote branch and tag, and every stash,
//! and with `--git-all` every commit and object. A payload cleaned from the
//! disk and still held there is one `git restore` away from coming back, and
//! a bare backup carrying it spreads it with every clone. ADR-0068.
//!
//! Git is read, never changed: plumbing only, the index never refreshed,
//! no lock taken, nothing fetched. The user's own git configuration is not
//! read, so it cannot change an answer, and the one setting of a
//! repository's own configuration that the commands used here could run,
//! `core.fsmonitor`, is switched off on every call.
//!
//! What counts is what the checks of the files on disk count, file kind by
//! file kind ([`judge`]): so the operator's own detection scripts, which carry
//! the indicator strings and are left alone on disk, are left alone in git.

use crate::checks::{is_build_config, is_js_source, is_truetype_name};
use crate::indicators::{Indicators, ManifestMatch};
use crate::manifest;
use crate::remote::{fake_font_magic, is_detection_tooling, is_hash_candidate};
use crate::verdict::{Finding, Kind, Verdict};
use crate::walk::Walk;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// How long one repository's default places may take before the rest of
/// them are left unread and said to be. `--git-all` has no limit.
pub const BUDGET: Duration = Duration::from_secs(120);

/// The places shown under a finding on the console; the report has all.
const SHOWN: usize = 5;

/// What to read.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Every commit and every object, not only what a checkout can bring
    /// back. `--git-all`.
    pub all: bool,
    /// Per repository, for the default places. `None` is no limit.
    pub budget: Option<Duration>,
    /// The program to run: `git`, or for a test one that is not there.
    pub program: &'static str,
}

/// Would the checks of the files on disk confirm this file, at this path in
/// a repository? The same questions, kind by kind: a build config or a
/// JavaScript source carrying an indicator, a `tasks.json` that runs on
/// folder open and carries one, a font whose bytes are not a font's, a file
/// known by its hash, a manifest that installs a campaign package or a
/// hijacked release, a name that is an indicator. Detection tooling is left
/// alone as it is on disk.
pub fn judge(path: &str, bytes: &[u8], ind: &Indicators) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let lower = name.to_ascii_lowercase();
    if ind.is_bad_filename(path) {
        return true;
    }
    if is_font(&lower) {
        let large =
            is_truetype_name(&lower) && bytes.len() as u64 > crate::checks::TRUETYPE_HASH_MAX;
        return fake_font_magic(path, bytes).is_some()
            || (!large && ind.known_file(bytes).is_some());
    }
    if is_tasks_json(path) {
        if ind.known_file(bytes).is_some() {
            return true;
        }
        let body = String::from_utf8_lossy(bytes);
        return crate::unescape::contains_decoded(&body, "folderOpen") && ind.has_strong(&body);
    }
    if is_build_config(name) {
        return crate::strip::plan(bytes, ind).is_some() || ind.known_file(bytes).is_some();
    }
    if is_js_source(name) {
        if is_detection_tooling(path) || bytes.iter().take(8192).any(|b| *b == 0) {
            return false;
        }
        let text = String::from_utf8_lossy(bytes);
        return ind.has_strong(&text) && crate::strip::plan(bytes, ind).is_some();
    }
    if manifest::is_manifest(name) {
        let text = String::from_utf8_lossy(bytes);
        let named = match ind.manifest_packages(name, &text) {
            ManifestMatch::Names(_) => true,
            ManifestMatch::Clean => false,
            ManifestMatch::Unreadable { text_match, .. } => text_match,
        };
        return named || !ind.manifest_releases(name, &text).is_empty();
    }
    is_hash_candidate(path) && ind.known_file(bytes).is_some()
}

/// Is this a file [`judge`] has to read? A name that is an indicator needs no
/// reading at all.
fn needs_reading(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let lower = name.to_ascii_lowercase();
    is_font(&lower)
        || is_tasks_json(path)
        || is_build_config(name)
        || is_js_source(name)
        || manifest::is_manifest(name)
        || is_hash_candidate(path)
}

fn is_font(lower: &str) -> bool {
    lower.ends_with(".woff") || lower.ends_with(".woff2") || is_truetype_name(lower)
}

fn is_tasks_json(path: &str) -> bool {
    path == ".vscode/tasks.json" || path.ends_with("/.vscode/tasks.json")
}

// --- running git ------------------------------------------------------------

/// Why git gave no answer.
#[derive(Debug)]
enum Failed {
    /// There is no `git` to run.
    NoGit,
    /// git refused: the repository belongs to another user.
    Owner,
    /// Anything else, with git's first line.
    Other(String),
}

/// One repository, as git is pointed at it.
struct Repo {
    program: &'static str,
    /// The git directory: `.git`, a worktree's or submodule's, or a bare
    /// repository.
    git_dir: PathBuf,
    /// The working tree, when there is one.
    work: Option<PathBuf>,
}

impl Repo {
    fn command(&self, args: &[&str]) -> Command {
        let mut c = Command::new(self.program);
        // Nothing inherited may point git elsewhere: another index, another
        // object store, another repository.
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("GIT_") {
                c.env_remove(key);
            }
        }
        c.args(["-c", "core.fsmonitor=false", "-c", "protocol.allow=never"])
            .args(args)
            .env("GIT_DIR", &self.git_dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .env("GIT_NO_LAZY_FETCH", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null());
        match &self.work {
            Some(work) => {
                c.env("GIT_WORK_TREE", work).current_dir(work);
            }
            None => {
                c.current_dir(&self.git_dir);
            }
        }
        c
    }

    fn run(&self, args: &[&str]) -> Result<Vec<u8>, Failed> {
        let out = match self.command(args).output() {
            Ok(out) => out,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(Failed::NoGit),
            Err(e) => return Err(Failed::Other(e.to_string())),
        };
        if out.status.success() {
            return Ok(out.stdout);
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("dubious ownership") {
            return Err(Failed::Owner);
        }
        let first = stderr
            .lines()
            .map(|l| l.trim().trim_start_matches("fatal: ").trim())
            .find(|l| !l.is_empty())
            .unwrap_or("git gave no reason")
            .to_string();
        Err(Failed::Other(first))
    }

    fn word(&self, args: &[&str]) -> Option<String> {
        self.run(args)
            .ok()
            .map(|out| String::from_utf8_lossy(&out).trim().to_string())
            .filter(|w| !w.is_empty())
    }
}

/// `git cat-file --batch`, asked one object at a time and read as it answers,
/// so that a repository is read with one process and one object in memory.
struct Batch {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

/// An object, as `cat-file --batch` gives it.
struct Object {
    id: String,
    kind: String,
    body: Vec<u8>,
}

impl Batch {
    fn open(repo: &Repo) -> Result<Self, Failed> {
        let mut child = match repo
            .command(&["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(Failed::NoGit),
            Err(e) => return Err(Failed::Other(e.to_string())),
        };
        let (Some(input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Failed::Other("git cat-file could not be started".into()));
        };
        Ok(Self {
            child,
            input,
            output: BufReader::new(output),
        })
    }

    /// The object a name resolves to, or `None` when it resolves to nothing.
    fn get(&mut self, name: &str) -> io::Result<Option<Object>> {
        if name.contains('\n') {
            return Ok(None);
        }
        self.input.write_all(name.as_bytes())?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        let mut header = String::new();
        if self.output.read_line(&mut header)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "git cat-file stopped answering",
            ));
        }
        let header = header.trim_end_matches('\n');
        if header.ends_with(" missing") || header.ends_with(" ambiguous") {
            return Ok(None);
        }
        let mut fields = header.rsplitn(3, ' ');
        let size = fields
            .next()
            .and_then(|n| n.parse::<usize>().ok())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, header.to_string()))?;
        let kind = fields.next().unwrap_or_default().to_string();
        let id = fields.next().unwrap_or_default().to_string();
        let mut body = vec![0u8; size];
        self.output.read_exact(&mut body)?;
        let mut newline = [0u8; 1];
        self.output.read_exact(&mut newline)?;
        Ok(Some(Object { id, kind, body }))
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The entries of a tree object: mode, name, object ID in hex.
fn tree_entries(body: &[u8], id_len: usize) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut rest = body;
    while !rest.is_empty() {
        let Some(space) = rest.iter().position(|b| *b == b' ') else {
            break;
        };
        let Some(nul) = rest.iter().position(|b| *b == 0) else {
            break;
        };
        let (Some(mode), Some(name), Some(raw)) = (
            rest.get(..space),
            rest.get(space + 1..nul),
            rest.get(nul + 1..nul + 1 + id_len),
        ) else {
            break;
        };
        let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        out.push((
            String::from_utf8_lossy(mode).into_owned(),
            String::from_utf8_lossy(name).into_owned(),
            hex,
        ));
        rest = rest.get(nul + 1 + id_len..).unwrap_or_default();
    }
    out
}

// --- reading one repository ---------------------------------------------------

/// The time ran out part way.
struct Stopped;

/// What reading one repository's places is doing, and has done.
struct Reader<'a> {
    batch: Batch,
    ind: &'a Indicators,
    id_len: usize,
    deadline: Option<Instant>,
    /// The carriers under each (tree, path prefix) already read.
    trees: HashMap<(String, String), Rc<Vec<String>>>,
    /// Each (blob, path) already judged.
    blobs: HashMap<(String, String), bool>,
    /// Objects read, for the `--git-all` count.
    read: usize,
    /// Told every 1,024 objects, so a long repository still shows it is
    /// moving.
    tick: &'a mut dyn FnMut(),
}

impl Reader<'_> {
    fn object(&mut self, name: &str) -> Result<Option<Object>, Stopped> {
        if self.deadline.is_some_and(|d| Instant::now() >= d) {
            return Err(Stopped);
        }
        self.read += 1;
        if self.read.is_multiple_of(1024) {
            (self.tick)();
        }
        // A repository git cannot read part of is read as far as it can be.
        Ok(self.batch.get(name).ok().flatten())
    }

    /// Does this blob, at this path, carry the payload?
    fn blob(&mut self, id: &str, path: &str) -> Result<bool, Stopped> {
        let key = (id.to_string(), path.to_string());
        if let Some(known) = self.blobs.get(&key) {
            return Ok(*known);
        }
        let carries = if self.ind.is_bad_filename(path) {
            true
        } else if needs_reading(path) {
            match self.object(id)? {
                Some(o) if o.kind == "blob" => judge(path, &o.body, self.ind),
                _ => false,
            }
        } else {
            false
        };
        self.blobs.insert(key, carries);
        Ok(carries)
    }

    /// Every path under this tree that carries the payload.
    fn tree(&mut self, name: &str, prefix: &str) -> Result<Rc<Vec<String>>, Stopped> {
        let Some(o) = self.object(name)? else {
            return Ok(Rc::new(Vec::new()));
        };
        if o.kind != "tree" {
            return Ok(Rc::new(Vec::new()));
        }
        let key = (o.id.clone(), prefix.to_string());
        if let Some(known) = self.trees.get(&key) {
            return Ok(Rc::clone(known));
        }
        let mut found = Vec::new();
        for (mode, entry, id) in tree_entries(&o.body, self.id_len) {
            let path = format!("{prefix}{entry}");
            match mode.as_str() {
                "40000" => found.extend(self.tree(&id, &format!("{path}/"))?.iter().cloned()),
                // A submodule's commit and a symbolic link hold no file here.
                "160000" | "120000" => {}
                _ => {
                    if self.blob(&id, &path)? {
                        found.push(path);
                    }
                }
            }
        }
        let found = Rc::new(found);
        self.trees.insert(key, Rc::clone(&found));
        Ok(found)
    }
}

/// One place in a repository, and what in it carries the payload.
struct Place {
    label: String,
    paths: Vec<String>,
}

/// What one repository held.
#[derive(Default)]
struct Held {
    places: Vec<Place>,
    refs: usize,
    refs_read: usize,
    stashes: usize,
    objects: usize,
    stopped: bool,
}

/// The branch and tag names a repository has, as (full name, label).
fn refs(repo: &Repo, bare: bool) -> Vec<(String, String)> {
    let Ok(out) = repo.run(&[
        "for-each-ref",
        "--format=%(refname)",
        "refs/heads/",
        "refs/remotes/",
        "refs/tags/",
    ]) else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out)
        .lines()
        .filter(|name| !name.ends_with("/HEAD"))
        .filter_map(|name| {
            let label = if bare {
                format!("bare repository: {name}")
            } else if let Some(branch) = name.strip_prefix("refs/heads/") {
                format!("branch {branch}")
            } else if let Some(branch) = name.strip_prefix("refs/remotes/") {
                format!("branch {branch}")
            } else {
                format!("tag {}", name.strip_prefix("refs/tags/")?)
            };
            Some((name.to_string(), label))
        })
        .collect()
}

/// The (blob, path) pairs staged in a work tree that differ from its last
/// commit, or every staged file when there is no commit yet.
fn staged(repo: &Repo, has_head: bool) -> Vec<(String, String)> {
    if has_head {
        let Ok(out) = repo.run(&[
            "diff-index",
            "--cached",
            "--raw",
            "-z",
            "--no-renames",
            "HEAD",
        ]) else {
            return Vec::new();
        };
        // ":<mode> <mode> <old> <new> <status>" NUL "<path>" NUL, repeated.
        let parts: Vec<&[u8]> = out.split(|b| *b == 0).collect();
        parts
            .chunks(2)
            .filter_map(|pair| {
                let meta = String::from_utf8_lossy(pair.first()?);
                let path = String::from_utf8_lossy(pair.get(1)?).into_owned();
                let new = meta.split(' ').nth(3)?.to_string();
                (!new.bytes().all(|b| b == b'0')).then_some((new, path))
            })
            .collect()
    } else {
        let Ok(out) = repo.run(&["ls-files", "-s", "-z"]) else {
            return Vec::new();
        };
        // "<mode> <blob> <stage>" TAB "<path>", NUL-separated.
        out.split(|b| *b == 0)
            .filter_map(|entry| {
                let entry = String::from_utf8_lossy(entry);
                let (meta, path) = entry.split_once('\t')?;
                let blob = meta.split(' ').nth(1)?.to_string();
                Some((blob, path.to_string()))
            })
            .collect()
    }
}

/// Read every place of one repository. `with_refs` is false for a second
/// work tree of a repository already read, whose branches are the same.
fn read_places(
    repo: &Repo,
    ind: &Indicators,
    with_refs: bool,
    options: Options,
    tick: &mut dyn FnMut(),
) -> Result<Held, Failed> {
    let id_len = match repo.word(&["rev-parse", "--show-object-format"]).as_deref() {
        Some("sha256") => 32,
        _ => 20,
    };
    let mut reader = Reader {
        batch: Batch::open(repo)?,
        ind,
        id_len,
        deadline: options.budget.map(|b| Instant::now() + b),
        trees: HashMap::new(),
        blobs: HashMap::new(),
        read: 0,
        tick,
    };
    let mut read = Held::default();
    let bare = repo.work.is_none();
    let names = if with_refs {
        refs(repo, bare)
    } else {
        Vec::new()
    };
    read.refs = names.len();

    let outcome = (|| -> Result<(), Stopped> {
        let mut has_head = false;
        if !bare {
            let branch = repo
                .word(&["symbolic-ref", "-q", "--short", "HEAD"])
                .unwrap_or_else(|| "detached".into());
            if let Some(head) = reader.object("HEAD^{tree}")? {
                has_head = true;
                let paths = reader.tree(&head.id, "")?.to_vec();
                read.places.push(Place {
                    label: format!("last commit ({branch})"),
                    paths,
                });
            }
            let mut paths = Vec::new();
            for (blob, path) in staged(repo, has_head) {
                if reader.blob(&blob, &path)? {
                    paths.push(path);
                }
            }
            read.places.push(Place {
                label: "staged".into(),
                paths,
            });
        }
        for (name, label) in &names {
            let paths = reader.tree(&format!("{name}^{{tree}}"), "")?.to_vec();
            read.refs_read += 1;
            read.places.push(Place {
                label: label.clone(),
                paths,
            });
        }
        if with_refs
            && repo
                .word(&["rev-parse", "-q", "--verify", "refs/stash"])
                .is_some()
        {
            let list = repo
                .run(&["log", "-g", "--format=%H", "refs/stash"])
                .map(|out| String::from_utf8_lossy(&out).into_owned())
                .unwrap_or_default();
            for (n, commit) in list.lines().filter(|l| !l.is_empty()).enumerate() {
                read.stashes += 1;
                let mut paths = Vec::new();
                // What the stash keeps of the work tree, of the index, and of
                // the untracked files.
                for tree in ["^{tree}", "^2^{tree}", "^3^{tree}"] {
                    for path in reader.tree(&format!("{commit}{tree}"), "")?.iter() {
                        if !paths.contains(path) {
                            paths.push(path.clone());
                        }
                    }
                }
                read.places.push(Place {
                    label: format!("stash@{{{n}}}"),
                    paths,
                });
            }
        }
        Ok(())
    })();
    if outcome.is_err() {
        read.stopped = true;
    }
    if options.all && !read.stopped && with_refs {
        read_everything(repo, &mut reader, &mut read);
    }
    read.objects = reader.read;
    read.places.retain(|p| !p.paths.is_empty());
    Ok(read)
}

/// `--git-all`: every commit any ref or reflog reaches, with the path each
/// file had, and then every object nothing reaches, read as JavaScript.
fn read_everything(repo: &Repo, reader: &mut Reader, read: &mut Held) {
    let Ok(listing) = repo.run(&["rev-list", "--objects", "--all", "--reflog"]) else {
        return;
    };
    let listing = String::from_utf8_lossy(&listing).into_owned();
    let mut reachable: HashSet<String> = HashSet::new();
    let mut carriers: Vec<(String, String)> = Vec::new();
    for line in listing.lines() {
        let (id, path) = line.split_once(' ').unwrap_or((line, ""));
        reachable.insert(id.to_string());
        if path.is_empty() || !(needs_reading(path) || reader.ind.is_bad_filename(path)) {
            continue;
        }
        if reader.blob(id, path).unwrap_or(false) {
            carriers.push((id.to_string(), path.to_string()));
        }
    }
    // Under the commit that first brought each one in.
    let mut by_commit: Vec<(String, Vec<String>)> = Vec::new();
    for (id, path) in carriers {
        let commit = repo
            .word(&[
                "log",
                "--all",
                "--reflog",
                "--format=%h",
                "--reverse",
                &format!("--find-object={id}"),
            ])
            .and_then(|out| out.lines().next().map(str::to_owned))
            .unwrap_or_else(|| id.chars().take(7).collect());
        let label = format!("earlier commit {commit}");
        match by_commit.iter_mut().find(|(l, _)| *l == label) {
            Some((_, paths)) => paths.push(path),
            None => by_commit.push((label, vec![path])),
        }
    }
    read.places.extend(
        by_commit
            .into_iter()
            .map(|(label, paths)| Place { label, paths }),
    );
    let Ok(all) = repo.run(&[
        "cat-file",
        "--batch-all-objects",
        "--batch-check=%(objectname) %(objecttype)",
    ]) else {
        return;
    };
    for line in String::from_utf8_lossy(&all).lines() {
        let Some((id, "blob")) = line.split_once(' ') else {
            continue;
        };
        if reachable.contains(id) {
            continue;
        }
        if reader.blob(id, "object.js").unwrap_or(false) {
            read.places.push(Place {
                label: format!(
                    "an object no branch or commit reaches ({})",
                    id.get(..7).unwrap_or(id)
                ),
                // Nothing names it, so there is no path to give.
                paths: vec![String::new()],
            });
        }
    }
}

// --- the check ----------------------------------------------------------------

/// Where a `.git` file points: `gitdir: <path>`, relative to the file.
fn pointed_at(file: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(file).ok()?;
    let target = text.lines().next()?.strip_prefix("gitdir:")?.trim();
    let target = Path::new(target);
    let full = if target.is_absolute() {
        target.to_path_buf()
    } else {
        file.parent()?.join(target)
    };
    full.is_dir().then_some(full)
}

/// The repository a git directory belongs to: its `commondir` when it is a
/// worktree's, itself otherwise.
fn common_dir(git_dir: &Path) -> PathBuf {
    let dir = fs::read_to_string(git_dir.join("commondir"))
        .ok()
        .map(|t| {
            let t = t.trim();
            let p = Path::new(t);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                git_dir.join(p)
            }
        })
        .unwrap_or_else(|| git_dir.to_path_buf());
    fs::canonicalize(&dir).unwrap_or(dir)
}

fn dead_pointers(n: usize) -> String {
    if n == 1 {
        "1 .git file points to a repository that is not there".into()
    } else {
        format!(
            "{} .git files point to a repository that is not there",
            crate::screen::thousands(n)
        )
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{} {many}", crate::screen::thousands(n))
    }
}

/// Read every repository the walk found. `tell` is told (done, of) before
/// each one.
pub fn history(
    walk: &Walk,
    ind: &Indicators,
    options: Options,
    v: &mut Verdict,
    tell: &mut dyn FnMut(usize, usize),
) {
    v.section("Git repositories");
    // (git directory, work tree, where it is shown)
    let mut found: Vec<(PathBuf, Option<PathBuf>, PathBuf)> = Vec::new();
    for dir in &walk.git_dirs {
        let is_dot_git = dir.file_name().and_then(|n| n.to_str()) == Some(".git");
        let work = is_dot_git
            .then(|| dir.parent().map(Path::to_path_buf))
            .flatten();
        let shown = work.clone().unwrap_or_else(|| dir.clone());
        found.push((dir.clone(), work, shown));
    }
    let mut dead = 0usize;
    for file in &walk.git_files {
        match pointed_at(file) {
            Some(git_dir) => {
                let work = file.parent().map(Path::to_path_buf);
                let shown = work.clone().unwrap_or_else(|| file.clone());
                found.push((git_dir, work, shown));
            }
            None => dead += 1,
        }
    }
    if found.is_empty() {
        if dead > 0 {
            v.push(Finding::info(dead_pointers(dead)));
        }
        v.push(Finding::ok("no git repositories found"));
        return;
    }

    let total = found.len();
    let mut seen_common: HashSet<PathBuf> = HashSet::new();
    let (mut repos, mut refs, mut stashes, mut objects, mut confirmed) = (0, 0, 0, 0, 0);
    let mut stopped = 0usize;
    for (n, (git_dir, work, shown)) in found.into_iter().enumerate() {
        tell(n, total);
        let repo = Repo {
            program: options.program,
            git_dir: git_dir.clone(),
            work,
        };
        if let Err(failed) = repo.run(&["rev-parse", "--git-dir"]) {
            match failed {
                Failed::NoGit => {
                    v.push(Finding::review(format!(
                        "git is not installed, so the history of {} was not read",
                        plural(total, "git repository", "git repositories")
                    )));
                    return;
                }
                Failed::Owner => v.push(
                    Finding::review(format!(
                        "git refused to read this repository (it belongs to another user), so it was not checked: {}",
                        shown.display()
                    ))
                    .at(&shown),
                ),
                Failed::Other(why) => v.push(
                    Finding::review(format!(
                        "git could not read this repository ({why}), so it was not checked: {}",
                        shown.display()
                    ))
                    .at(&shown),
                ),
            }
            continue;
        }
        let with_refs = seen_common.insert(common_dir(&git_dir));
        let budget = if options.all { None } else { options.budget };
        let mut tick = || tell(n, total);
        let read = match read_places(
            &repo,
            ind,
            with_refs,
            Options { budget, ..options },
            &mut tick,
        ) {
            Ok(read) => read,
            Err(Failed::NoGit) => {
                v.push(Finding::review(format!(
                    "git is not installed, so the history of {} was not read",
                    plural(total, "git repository", "git repositories")
                )));
                return;
            }
            Err(Failed::Owner) => continue,
            Err(Failed::Other(why)) => {
                v.push(
                    Finding::review(format!(
                        "git could not read this repository ({why}), so it was not checked: {}",
                        shown.display()
                    ))
                    .at(&shown),
                );
                continue;
            }
        };
        repos += 1;
        refs += read.refs;
        stashes += read.stashes;
        objects += read.objects;
        if !read.places.is_empty() {
            confirmed += 1;
            for (i, place) in read.places.iter().enumerate() {
                let line = if place.paths.iter().all(String::is_empty) {
                    place.label.clone()
                } else {
                    format!("{}: {}", place.label, place.paths.join(", "))
                };
                if i < SHOWN {
                    v.detail(line);
                } else {
                    v.note(line);
                }
            }
            if read.places.len() > SHOWN {
                v.brief(format!(
                    "and {} more, in the report",
                    read.places.len() - SHOWN
                ));
            }
            let remedy = if repo.work.is_some() {
                "git keeps a copy, so a git restore, reset, checkout or stash pop puts the payload back after the files on disk are cleaned.\nCommit the cleaned files first. Then delete or rewrite each branch named, and drop each stash named."
            } else {
                "a bare repository: a clone of it, or a push from it, spreads the payload.\nDelete it, or keep it offline and labelled."
            };
            v.push(
                Finding::hit(
                    Kind::GitHistory,
                    format!(
                        "the payload is in this repository's git history: {}",
                        shown.display()
                    ),
                )
                .at(&shown)
                .with_remedy(remedy),
            );
        }
        if read.stopped {
            stopped += 1;
            let minutes = budget.map_or(0, |b| b.as_secs() / 60);
            v.push(
                Finding::review(format!(
                    "stopped after {minutes} minutes, {} of {} branches read, so the rest were not checked: {}",
                    read.refs_read,
                    read.refs,
                    shown.display()
                ))
                .at(&shown),
            );
        }
    }
    tell(total, total);
    if dead > 0 {
        v.push(Finding::info(dead_pointers(dead)));
    }
    // "None holds the payload" only when every one was read to the end.
    if confirmed == 0 && stopped == 0 && repos > 0 {
        let what = if options.all {
            format!(
                "every commit and object, {} read",
                plural(objects, "object", "objects")
            )
        } else {
            format!(
                "last commits, staged files, {}, {}",
                plural(refs, "branch or tag", "branches and tags"),
                plural(stashes, "stash", "stashes")
            )
        };
        v.push(Finding::ok(format!(
            "{} read ({what}), none holds the payload",
            plural(repos, "git repository", "git repositories")
        )));
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::remote::fixture::{git, git_out};
    use crate::verdict::{Entry, Level};

    const STRONG: &str = "MARKER-ALPHA-GIT-LOCAL";

    fn ind() -> Indicators {
        Indicators {
            strong: vec![STRONG.into()],
            weak: vec!["WEAK-GIT-LOCAL".into()],
            ..Indicators::default()
        }
    }

    fn infected() -> String {
        format!("export default {{}}\n{}{STRONG}\n", " ".repeat(80))
    }

    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("prc-git-local-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        /// A repository with its first commit on main.
        fn repo(&self, name: &str, files: &[(&str, &str)]) -> PathBuf {
            let dir = self.0.join(name);
            fs::create_dir_all(&dir).unwrap();
            git(&dir, &["init", "-q", "-b", "main"]);
            self.commit(&dir, files);
            dir
        }

        fn commit(&self, dir: &Path, files: &[(&str, &str)]) {
            write(dir, files);
            git(dir, &["add", "-A"]);
            git(dir, &["commit", "-q", "--allow-empty", "-m", "change"]);
        }

        fn check(&self, options: Options) -> Verdict {
            let walk = crate::walk::walk(std::slice::from_ref(&self.0));
            let mut v = Verdict::new();
            history(&walk, &ind(), options, &mut v, &mut |_, _| {});
            v
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write(dir: &Path, files: &[(&str, &str)]) {
        for (path, body) in files {
            let file = dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, body).unwrap();
        }
    }

    fn default() -> Options {
        Options {
            all: false,
            budget: Some(BUDGET),
            program: "git",
        }
    }

    fn details(v: &Verdict) -> Vec<String> {
        v.entries()
            .iter()
            .filter_map(|e| match e {
                Entry::Detail(d) | Entry::Note(d) => Some(d.clone()),
                _ => None,
            })
            .collect()
    }

    fn said(v: &Verdict, level: Level) -> Vec<String> {
        v.findings()
            .filter(|f| f.level == level)
            .map(|f| f.message.clone())
            .collect()
    }

    #[test]
    fn the_last_commit_staged_files_branches_tags_and_stashes_are_each_named() {
        let t = Tree::new("places");
        let shop = t.repo("shop", &[("postcss.config.mjs", &infected())]);
        // Cleaned on disk, not committed: a git restore puts it back.
        write(&shop, &[("postcss.config.mjs", "export default {}\n")]);
        git(&shop, &["checkout", "-q", "-b", "feature/login"]);
        t.commit(&shop, &[("src/routes/jobs.js", &infected())]);
        git(&shop, &["tag", "v1.2.0"]);
        git(&shop, &["checkout", "-q", "-f", "main"]);
        write(&shop, &[("postcss.config.mjs", "export default {}\n")]);
        write(&shop, &[("eslint.config.js", &infected())]);
        git(&shop, &["stash", "push", "-q", "-u"]);
        write(&shop, &[("src/app.js", &infected())]);
        git(&shop, &["add", "src/app.js"]);
        write(&shop, &[("src/app.js", "export {}\n")]);

        let v = t.check(default());
        assert_eq!(v.hits(), 1, "{:?}", said(&v, Level::Hit));
        let found = details(&v);
        for want in [
            "last commit (main): postcss.config.mjs",
            "staged: src/app.js",
            "branch feature/login: src/routes/jobs.js",
            "tag v1.2.0: src/routes/jobs.js",
            // The stash keeps the index, where the last commit's copy still
            // is, and the untracked file.
            "stash@{0}: postcss.config.mjs, eslint.config.js",
        ] {
            assert!(found.iter().any(|d| d == want), "{want} not in {found:?}");
        }
        let hit = v.findings().find(|f| f.level == Level::Hit).unwrap();
        assert_eq!(hit.kind, Some(Kind::GitHistory));
        assert!(hit
            .message
            .starts_with("the payload is in this repository's git history: "));
    }

    #[test]
    fn a_bare_repository_and_a_clone_are_found_and_a_worktree_is_read_once() {
        let t = Tree::new("bare");
        let work = t.repo("work", &[("vite.config.js", &infected())]);
        git(&t.0, &["clone", "-q", "--bare", "work", "backups/shop.git"]);
        git(&t.0, &["clone", "-q", "work", "copy"]);
        git(&work, &["branch", "wt-branch"]);
        git(&work, &["worktree", "add", "-q", "../wt", "wt-branch"]);
        let walk = crate::walk::walk(std::slice::from_ref(&t.0));
        assert!(walk
            .git_dirs
            .iter()
            .any(|d| d.ends_with("backups/shop.git")));
        assert!(walk.git_files.iter().any(|f| f.ends_with("wt/.git")));
        assert!(
            !walk
                .files
                .iter()
                .any(|f| f.starts_with(t.0.join("backups"))),
            "a bare repository is not walked as files"
        );

        let v = t.check(default());
        let found = details(&v);
        assert!(found.contains(&"bare repository: refs/heads/main: vite.config.js".to_string()));
        assert!(
            found.contains(&"branch origin/main: vite.config.js".to_string()),
            "{found:?}"
        );
        // The worktree's own last commit, and the shared branches only once.
        assert!(found.contains(&"last commit (wt-branch): vite.config.js".to_string()));
        assert_eq!(
            found
                .iter()
                .filter(|d| *d == "branch wt-branch: vite.config.js")
                .count(),
            1,
            "{found:?}"
        );
        assert_eq!(
            v.hits(),
            4,
            "work, its worktree, the bare copy and the clone"
        );
        let bare = v
            .findings()
            .find(|f| f.message.ends_with("shop.git"))
            .unwrap();
        assert!(bare
            .remedy
            .as_deref()
            .unwrap()
            .starts_with("a bare repository"));
    }

    #[test]
    fn fifty_branches_on_one_tree_read_each_object_once() {
        let t = Tree::new("dedupe");
        let files: Vec<(String, String)> = (0..40)
            .map(|i| (format!("src/m{i}.js"), format!("export const m{i} = {i}\n")))
            .collect();
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(p, b)| (p.as_str(), b.as_str()))
            .collect();
        let shop = t.repo("shop", &refs);
        for i in 0..50 {
            git(&shop, &["branch", &format!("b{i}")]);
        }
        let repo = Repo {
            program: "git",
            git_dir: shop.join(".git"),
            work: Some(shop.clone()),
        };
        let read = read_places(&repo, &ind(), true, default(), &mut || {})
            .ok()
            .unwrap();
        assert_eq!(read.refs, 51);
        // HEAD's tree, each ref's tree once, the src tree and its 40 files
        // once: not 51 times each.
        assert!(read.objects <= 1 + 51 + 1 + 40 + 2, "{}", read.objects);
        assert!(read.places.is_empty());
    }

    #[test]
    fn detection_tooling_and_weak_signals_in_git_are_left_as_on_disk() {
        let t = Tree::new("tooling");
        t.repo(
            "tools",
            &[
                ("docs/sample.js", &infected()),
                ("scripts/polinrider-scan.sh", STRONG),
                ("src/rpc.js", "export const rpc = 'WEAK-GIT-LOCAL'\n"),
            ],
        );
        let v = t.check(default());
        assert_eq!(v.hits(), 0, "{:?}", said(&v, Level::Hit));
        assert_eq!(v.reviews(), 0, "{:?}", said(&v, Level::Review));
        assert_eq!(
            said(&v, Level::Ok),
            vec!["1 git repository read (last commits, staged files, 1 branch or tag, 0 stashes), none holds the payload"]
        );
    }

    #[test]
    fn judge_agrees_with_the_checks_of_the_files_on_disk() {
        let ind = ind();
        let mut font = b"var x = 1;".to_vec();
        font.extend_from_slice(STRONG.as_bytes());
        for (path, bytes, disk) in [
            ("postcss.config.mjs", infected().into_bytes(), true),
            ("src/routes/jobs.js", infected().into_bytes(), true),
            ("docs/sample.js", infected().into_bytes(), false),
            ("public/fonts/fa-solid-400.woff2", font.clone(), true),
            ("public/fonts/real.woff2", b"wOF2rest".to_vec(), false),
            (
                ".vscode/tasks.json",
                format!("{{\"runOn\": \"folderOpen\", \"command\": \"{STRONG}\"}}").into_bytes(),
                true,
            ),
            (
                ".vscode/tasks.json",
                b"{\"runOn\": \"folderOpen\", \"command\": \"npm i\"}".to_vec(),
                false,
            ),
            ("notes/readme.txt", infected().into_bytes(), false),
            ("src/plain.js", b"export const a = 1\n".to_vec(), false),
        ] {
            assert_eq!(judge(path, &bytes, &ind), disk, "{path}");
        }
    }

    #[test]
    fn with_git_all_earlier_commits_and_stray_objects_are_read() {
        let t = Tree::new("all");
        let shop = t.repo("shop", &[("eslint.config.js", &infected())]);
        t.commit(&shop, &[("eslint.config.js", "export default []\n")]);
        let blob = {
            let stray = shop.join("stray.tmp");
            // Not the same bytes as the committed file, or it would be the
            // same object, and reachable.
            fs::write(&stray, format!("// stray\n{}", infected())).unwrap();
            let id = git_out(&shop, &["hash-object", "-w", "stray.tmp"]);
            fs::remove_file(stray).unwrap();
            id
        };
        let v = t.check(default());
        assert_eq!(v.hits(), 0);
        let v = t.check(Options {
            all: true,
            ..default()
        });
        assert_eq!(v.hits(), 1);
        let found = details(&v);
        assert!(
            found
                .iter()
                .any(|d| d.starts_with("earlier commit ") && d.ends_with(": eslint.config.js")),
            "{found:?}"
        );
        assert!(
            found.contains(&format!(
                "an object no branch or commit reaches ({})",
                &blob[..7]
            )),
            "{found:?}"
        );
    }

    #[test]
    fn when_the_time_runs_out_the_rest_is_said_to_be_unread() {
        let t = Tree::new("budget");
        let shop = t.repo("shop", &[("README.txt", "shop\n")]);
        git(&shop, &["branch", "other"]);
        let v = t.check(Options {
            budget: Some(Duration::ZERO),
            ..default()
        });
        let reviews = said(&v, Level::Review);
        assert_eq!(reviews.len(), 1, "{reviews:?}");
        assert!(
            reviews[0].starts_with(
                "stopped after 0 minutes, 0 of 2 branches read, so the rest were not checked: "
            ),
            "{reviews:?}"
        );
    }

    #[test]
    fn no_git_a_broken_git_folder_and_a_dead_pointer_are_each_said() {
        let t = Tree::new("broken");
        t.repo("shop", &[("README.txt", "shop\n")]);
        let v = t.check(Options {
            program: "git-that-is-not-installed-anywhere",
            ..default()
        });
        assert_eq!(
            said(&v, Level::Review),
            vec!["git is not installed, so the history of 1 git repository was not read"]
        );

        let t = Tree::new("stub");
        write(
            &t.0,
            &[
                ("proj/.git/hooks/README", "x\n"),
                ("gone/.git", "gitdir: ../nowhere/.git/worktrees/gone\n"),
            ],
        );
        let v = t.check(default());
        let reviews = said(&v, Level::Review);
        assert_eq!(reviews.len(), 1, "{reviews:?}");
        assert!(
            reviews[0].starts_with("git could not read this repository ("),
            "{reviews:?}"
        );
        assert!(reviews[0].ends_with("proj"), "{reviews:?}");
        assert_eq!(
            said(&v, Level::Info),
            vec!["1 .git file points to a repository that is not there"]
        );
    }

    #[test]
    fn git_is_read_without_running_anything_or_changing_anything() {
        let t = Tree::new("inert");
        let shop = t.repo("shop", &[("postcss.config.mjs", &infected())]);
        // A program a repository's config names, which the commands used
        // here could start, and a replace ref hiding the payload's blob.
        let marker = t.0.join("ran");
        let script = t.0.join("monitor.sh");
        fs::write(
            &script,
            format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        }
        git(
            &shop,
            &["config", "core.fsmonitor", &script.display().to_string()],
        );
        let payload = git_out(&shop, &["rev-parse", "HEAD:postcss.config.mjs"]);
        fs::write(shop.join("clean.txt"), "export default {}\n").unwrap();
        let clean = git_out(&shop, &["hash-object", "-w", "clean.txt"]);
        fs::remove_file(shop.join("clean.txt")).unwrap();
        git(&shop, &["replace", &payload, &clean]);
        let _ = fs::remove_file(&marker);

        let snapshot = |dir: &Path| {
            let mut all: Vec<(PathBuf, Vec<u8>, Option<std::time::SystemTime>)> = Vec::new();
            let mut stack = vec![dir.join(".git")];
            while let Some(d) = stack.pop() {
                for e in fs::read_dir(&d).unwrap().flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else {
                        let m = fs::metadata(&p).ok().and_then(|m| m.modified().ok());
                        all.push((p.clone(), fs::read(&p).unwrap(), m));
                    }
                }
            }
            all.sort();
            all
        };
        let before = snapshot(&shop);
        let v = t.check(default());
        assert_eq!(v.hits(), 1, "the replace ref must not hide the payload");
        assert!(!marker.exists(), "the repository's fsmonitor program ran");
        assert!(before == snapshot(&shop), ".git changed");
    }

    #[test]
    fn a_tree_is_read_into_its_entries() {
        let mut body = b"100644 a.js\0".to_vec();
        body.extend([0xab; 20]);
        body.extend_from_slice(b"40000 src\0");
        body.extend([0x01; 20]);
        let entries = tree_entries(&body, 20);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, "100644");
        assert_eq!(entries[0].1, "a.js");
        assert_eq!(entries[0].2, "ab".repeat(20));
        assert_eq!(entries[1].1, "src");
    }
}
