//! One pruned walk of the filesystem, reused by every check.
//!
//! ADR-0025. The shell did this eight times, each written as
//! `-not -path '*/node_modules/*'`, which filters what `find` prints without
//! stopping it descending. A backup drive took six hours. Here the walk
//! happens once, directories on the prune list are never entered, and the
//! checks read the resulting list.
//!
//! `node_modules` is not walked at all. Campaign packages are caught by name
//! from manifests and lockfiles, and the payload lives in the project's own
//! config files and fonts rather than inside a dependency. That is a real
//! limitation, recorded in ADR-0025 and pinned by a conformance case. Each
//! `node_modules` is listed, so that a hijacked release installed in it can
//! be found by reading a few named files (ADR-0063).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Directory names never entered. Matched on the final component, so it
/// applies at any depth.
pub const PRUNED: &[&str] = &[
    "node_modules",
    ".git",
    ".Trash",
    ".cache",
    "__MACOSX",
    ".npm",
    ".pnpm-store",
    ".yarn",
    ".venv",
    "venv",
    "Library",
];

/// The result of walking the roots once.
#[derive(Debug, Default)]
pub struct Walk {
    /// Every regular file found, excluding anything under a pruned directory.
    pub files: Vec<PathBuf>,
    /// `.git` directories, and bare repositories under any name, listed but
    /// never entered, so hooks can still be read and history read through
    /// git without walking an object store. ADR-0068.
    pub git_dirs: Vec<PathBuf>,
    /// `.git` files, which point at a repository kept elsewhere: a worktree's
    /// or a submodule's. Also in `files`. ADR-0068.
    pub git_files: Vec<PathBuf>,
    /// Python virtual environments (`.venv`, `venv`), listed but never
    /// entered, so the startup files in their site-packages can still be
    /// read without walking every installed package.
    pub python_envs: Vec<PathBuf>,
    /// `node_modules` directories, listed but never entered, so the few files
    /// that say which releases are installed can be read by name without
    /// walking every package. ADR-0063.
    pub node_modules: Vec<PathBuf>,
    /// Roots that did not exist or could not be read. Reported rather than
    /// skipped silently: scanning nothing must not look like scanning clean.
    pub unreadable: Vec<PathBuf>,
}

/// What a walk leaves out.
#[derive(Debug, Clone)]
pub struct Options<'a> {
    /// Directory names never entered.
    pub prune: &'a [&'a str],
    /// One directory never entered, wherever it is: the quarantine. Without
    /// this, a quarantine placed under a scanned root is walked by the next
    /// scan, which finds every artifact it holds and reports the machine as
    /// still infected by its own evidence.
    pub skip: Option<PathBuf>,
    /// One file never listed: this run's own report, which is written while
    /// the walk goes on and may sit under a scanned root. Read, it would be
    /// a weak signal of its own making. ADR-0059.
    pub skip_file: Option<PathBuf>,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Self {
            prune: PRUNED,
            skip: None,
            skip_file: None,
        }
    }
}

impl<'a> Options<'a> {
    /// The usual prune list, and never enter `quarantine`. Resolved once here
    /// so that a relative path and the absolute one the walk meets compare
    /// equal. A quarantine that does not exist yet has nothing in it to skip.
    pub fn skipping(quarantine: &Path) -> Self {
        Self {
            prune: PRUNED,
            skip: fs::canonicalize(quarantine).ok(),
            skip_file: None,
        }
    }

    /// Never list `report` either. Resolved once, like the quarantine; a
    /// report that could not be created has nothing to skip.
    #[must_use]
    pub fn and_not(mut self, report: Option<&Path>) -> Self {
        self.skip_file = report.and_then(|r| fs::canonicalize(r).ok());
        self
    }

    fn excludes_file(&self, file: &Path) -> bool {
        self.skip_file.as_deref().is_some_and(|skip| {
            skip.file_name() == file.file_name() && fs::canonicalize(file).is_ok_and(|f| f == skip)
        })
    }

    fn excludes(&self, dir: &Path) -> bool {
        let Some(name) = dir.file_name() else {
            return false;
        };
        if name.to_str().is_some_and(|n| {
            // Any quarantine, this run's or an earlier one's: 1.x and 2.0
            // both name them this way.
            self.prune.contains(&n) || n.starts_with("polinrider-quarantine")
        }) {
            return true;
        }
        // The name is compared first because it is free; resolving every
        // directory on the disk to compare paths is not.
        self.skip.as_deref().is_some_and(|skip| {
            skip.file_name() == Some(name) && fs::canonicalize(dir).is_ok_and(|d| d == skip)
        })
    }
}

/// Walk every root once. Never follows symlinks: a link into `/` would
/// otherwise turn a project scan into a whole-disk scan, and a link pointing
/// outside the tree is not part of what the caller asked to scan.
pub fn walk(roots: &[PathBuf]) -> Walk {
    walk_with(roots, &Options::default())
}

pub fn walk_with(roots: &[PathBuf], options: &Options) -> Walk {
    walk_watched(roots, options, &mut |_| {})
}

/// How many files are listed between two calls to a walk's watcher.
const TICK: usize = 1024;

/// The same walk, telling `watch` how many files it has listed so far, every
/// thousand or so. A home folder is hundreds of thousands of files, and a
/// walk that says nothing until it is done reads as a hang.
pub fn walk_watched(roots: &[PathBuf], options: &Options, watch: &mut dyn FnMut(usize)) -> Walk {
    walk_seeing(roots, options, &mut |seen| {
        if let Seen::Listed(n) = seen {
            watch(n);
        }
    })
}

/// What a walk tells its visitor, as it happens.
#[derive(Debug, Clone, Copy)]
pub enum Seen<'a> {
    /// How many files are listed so far, every thousand or so.
    Listed(usize),
    /// A file, the moment it is listed.
    File(&'a Path),
    /// A `.git` directory, listed and not entered.
    GitDir(&'a Path),
}

/// The same walk, showing `see` each file and `.git` directory as it is
/// found, so a check can read the few files that matter most without
/// waiting for the walk of a whole drive to finish. ADR-0059.
pub fn walk_seeing(roots: &[PathBuf], options: &Options, see: &mut dyn FnMut(Seen)) -> Walk {
    let mut out = Walk::default();
    for root in roots {
        match fs::symlink_metadata(root) {
            Ok(m) if m.is_dir() => descend(root, &mut out, options, see),
            Ok(_) => {
                see(Seen::File(root));
                out.files.push(root.clone());
            }
            Err(_) => out.unreadable.push(root.clone()),
        }
    }
    out.files.sort();
    out.git_dirs.sort();
    out.git_files.sort();
    out.python_envs.sort();
    out.node_modules.sort();
    out
}

fn descend(dir: &Path, out: &mut Walk, options: &Options, see: &mut dyn FnMut(Seen)) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        // An unreadable subdirectory is noted, not fatal: a scan of a backup
        // drive will hit directories the current user cannot open.
        Err(_) => {
            out.unreadable.push(dir.to_path_buf());
            return;
        }
    };
    let entries: Vec<(fs::DirEntry, fs::FileType)> = entries
        .flatten()
        .filter_map(|e| e.file_type().ok().map(|t| (e, t)))
        .collect();

    // A bare repository, whatever it is called: a HEAD file beside objects
    // and refs folders. Its objects are compressed and mean nothing read as
    // files, so it is listed for git to read and not entered. ADR-0068.
    let has = |name: &str, dir: bool| {
        entries
            .iter()
            .any(|(e, t)| e.file_name() == name && if dir { t.is_dir() } else { t.is_file() })
    };
    if has("HEAD", false) && has("objects", true) && has("refs", true) {
        see(Seen::GitDir(dir));
        out.git_dirs.push(dir.to_path_buf());
        return;
    }

    for (entry, meta) in entries {
        let path = entry.path();

        if meta.is_symlink() {
            continue;
        }
        if meta.is_dir() {
            if path.file_name().and_then(|n| n.to_str()) == Some(".git") {
                see(Seen::GitDir(&path));
                out.git_dirs.push(path);
                continue;
            }
            if options.excludes(&path) {
                match path.file_name().and_then(|n| n.to_str()) {
                    Some(".venv" | "venv") => out.python_envs.push(path),
                    Some("node_modules") => out.node_modules.push(path),
                    _ => {}
                }
                continue;
            }
            descend(&path, out, options, see);
        } else if meta.is_file() {
            if options.excludes_file(&path) {
                continue;
            }
            see(Seen::File(&path));
            if path.file_name().and_then(|n| n.to_str()) == Some(".git") {
                out.git_files.push(path.clone());
            }
            out.files.push(path);
            if out.files.len().is_multiple_of(TICK) {
                see(Seen::Listed(out.files.len()));
            }
        }
    }
}

impl Walk {
    /// Files whose name matches a predicate. The checks use this instead of
    /// walking again.
    pub fn by_name<'a>(
        &'a self,
        pred: impl Fn(&str) -> bool + 'a,
    ) -> impl Iterator<Item = &'a PathBuf> {
        self.files
            .iter()
            .filter(move |p| p.file_name().and_then(|n| n.to_str()).is_some_and(&pred))
    }

    /// Executable, non-sample hook files in every `.git/hooks` found.
    pub fn git_hooks(&self) -> Vec<PathBuf> {
        let mut hooks: Vec<PathBuf> = self.git_dirs.iter().flat_map(|g| hooks_in(g)).collect();
        hooks.sort();
        hooks
    }
}

/// Executable, non-sample hook files in one `.git/hooks`, sorted.
pub fn hooks_in(git_dir: &Path) -> Vec<PathBuf> {
    let mut hooks = Vec::new();
    let Ok(entries) = fs::read_dir(git_dir.join("hooks")) else {
        return hooks;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) == Some("sample") {
            continue;
        }
        if p.is_file() && is_executable(&p) {
            hooks.push(p);
        }
    }
    hooks.sort();
    hooks
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(p)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_p: &Path) -> bool {
    true
}

/// Create a directory and everything above it, for callers that need one.
pub fn ensure_dir(p: &Path) -> io::Result<()> {
    fs::create_dir_all(p)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn tree(name: &str, files: &[&str]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("prc-walk-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for f in files {
            let p = root.join(f);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).expect("mkdir");
            }
            fs::write(&p, b"x").expect("write");
        }
        root
    }

    #[test]
    fn pruned_directories_are_never_entered() {
        // The whole point of ADR-0025. A file inside node_modules must not
        // appear, no matter how deep.
        let root = tree(
            "prune",
            &[
                "proj/postcss.config.mjs",
                "proj/node_modules/evil/deep/deeper/payload.js",
                "proj/.cache/thing.js",
                "proj/src/index.js",
            ],
        );
        let w = walk(std::slice::from_ref(&root));
        let names: Vec<String> = w.files.iter().map(|p| p.display().to_string()).collect();
        assert!(names.iter().any(|n| n.ends_with("postcss.config.mjs")));
        assert!(names.iter().any(|n| n.ends_with("src/index.js")));
        assert!(
            !names.iter().any(|n| n.contains("node_modules")),
            "{names:?}"
        );
        assert!(!names.iter().any(|n| n.contains(".cache")), "{names:?}");
        // Listed, never entered, so its install record can be read by name.
        assert_eq!(w.node_modules, [root.join("proj/node_modules")]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_quarantine_under_a_scanned_root_is_never_walked() {
        // The second scan of a guided session used to find the artifacts the
        // first one had just quarantined, and report them as still present.
        let root = tree(
            "quarantine",
            &[
                "proj/a.js",
                "proj/evidence-here/files/proj/fake.woff2",
                "polinrider-quarantine-20260101T000000Z/files/proj/fake.woff2",
            ],
        );
        let w = walk_with(
            std::slice::from_ref(&root),
            &Options::skipping(&root.join("proj/evidence-here")),
        );
        let names: Vec<String> = w.files.iter().map(|p| p.display().to_string()).collect();
        assert_eq!(names.len(), 1, "{names:?}");
        assert!(names[0].ends_with("proj/a.js"));

        // Without being told, the oddly named one is walked like anything else.
        let w = walk(std::slice::from_ref(&root));
        assert_eq!(w.files.len(), 2);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn git_is_listed_but_not_walked() {
        // Hooks still have to be readable without walking an object store.
        let root = tree(
            "git",
            &[
                "proj/.git/hooks/pre-commit",
                "proj/.git/objects/aa/bb",
                "proj/a.js",
            ],
        );
        let w = walk(std::slice::from_ref(&root));
        assert_eq!(w.git_dirs.len(), 1, "the .git directory should be listed");
        assert!(
            !w.files
                .iter()
                .any(|p| p.display().to_string().contains("/.git/")),
            "nothing inside .git should be in the file list"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_root_is_reported_not_silently_skipped() {
        // Scanning nothing must never look like scanning clean.
        let w = walk(&[PathBuf::from("/definitely/not/here/at/all")]);
        assert_eq!(w.unreadable.len(), 1);
        assert!(w.files.is_empty());
    }

    #[test]
    fn symlinks_are_not_followed() {
        // A link to / would turn a project scan into a whole-disk scan.
        let root = tree("symlink", &["proj/real.js"]);
        #[cfg(unix)]
        {
            let link = root.join("proj/escape");
            let _ = std::os::unix::fs::symlink("/etc", &link);
            let w = walk(std::slice::from_ref(&root));
            assert!(
                !w.files
                    .iter()
                    .any(|p| p.display().to_string().contains("passwd")),
                "must not have followed the link out of the tree"
            );
        }
        let _ = fs::remove_dir_all(&root);
    }
}
