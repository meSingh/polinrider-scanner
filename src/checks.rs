//! The filesystem checks.
//!
//! Two of them, the implant and the git configuration, have a half that reads
//! the machine rather than the disk. That half arrives as an optional
//! [`Host`]: `None` under `--fs-only`, and the check says what it did not read.
//!
//! Every message here is matched verbatim by the conformance corpus, which is
//! the specification both implementations answer to. Changing a string is a
//! behaviour change, and the corpus will say so.

use crate::host::Host;
use crate::host_checks::{self, sh_quote, ProcessCheck};
use crate::indicators::{Indicators, ManifestMatch};
use crate::manifest;
use crate::quarantine::{Apply, DryRun, Quarantine};
use crate::strip::{self, Plan};
use crate::verdict::{Finding, Kind, Verdict};
use crate::walk::Walk;
use std::fs;
use std::path::{Path, PathBuf};

/// Where confirmed artifacts go, if anywhere. The two variants are different
/// types, so a dry run has no way to move a file. See `quarantine`.
pub enum Sink<'a> {
    Dry(&'a Quarantine<DryRun>),
    Apply(&'a mut Quarantine<Apply>),
}

impl Sink<'_> {
    /// Where this run's quarantine is, or would be.
    pub fn root(&self) -> &Path {
        match self {
            Sink::Dry(q) => q.root(),
            Sink::Apply(q) => q.root(),
        }
    }

    /// Record a confirmed artifact. Returns the line to print under it.
    pub(crate) fn take(&mut self, path: &Path, reason: &str) -> String {
        match self {
            Sink::Dry(q) => q.would_take(path).line(),
            Sink::Apply(q) => match q.take(path, reason) {
                Ok(o) => o.line(),
                Err(e) => format!("QUARANTINE FAILED ({e}): {}", path.display()),
            },
        }
    }

    /// Cut a payload out of a file, keeping the original. Returns the line to
    /// print under the finding. A dry run has no way to do it: `strip` does
    /// not exist on `Quarantine<DryRun>`.
    fn strip(&mut self, path: &Path, plan: &Plan) -> String {
        let outcome = match self {
            Sink::Dry(q) => q.would_strip(path).line(),
            Sink::Apply(q) => match q.strip(path, &plan.keep, "stripped-config") {
                Ok(o) => o.line(),
                Err(e) => {
                    return format!(
                        "STRIP FAILED ({e}), the file is unchanged: {}",
                        path.display()
                    )
                }
            },
        };
        let verb = match self {
            Sink::Dry(_) => "would strip",
            Sink::Apply(_) => "stripped",
        };
        format!(
            "{verb} {} bytes appended after line {}\n{outcome}",
            plan.removed, plan.last_kept_line
        )
    }
}

/// The config filenames the campaign appends to.
pub(crate) fn is_build_config(name: &str) -> bool {
    const STEMS: &[&str] = &[
        "postcss.config",
        "tailwind.config",
        "eslint.config",
        "vite.config",
        "next.config",
        "rollup.config",
        "webpack.config",
        "babel.config",
        "gridsome.config",
        "vue.config",
    ];
    name == "truffle.js"
        || STEMS
            .iter()
            .any(|s| name.starts_with(s) && name[s.len()..].starts_with('.'))
}

/// JavaScript and TypeScript source. Read for confirmed indicators as well as
/// the build configs: the campaign appends the same payload, behind the same
/// padding, to route files, `App.js`, `jest.config.js` and others the config
/// names miss. ADR-0058.
pub(crate) fn is_js_source(name: &str) -> bool {
    const EXTENSIONS: &[&str] = &[".js", ".mjs", ".cjs", ".ts", ".jsx", ".tsx"];
    let lower = name.to_ascii_lowercase();
    EXTENSIONS.iter().any(|e| lower.ends_with(e))
}

/// Is this path detection tooling, judged relative to the root it was found
/// under, as the tooling patterns are written for a repository's own paths?
fn is_tooling(path: &Path, roots: &[PathBuf]) -> bool {
    let relative = roots
        .iter()
        .filter_map(|r| path.strip_prefix(r).ok())
        .min_by_key(|p| p.as_os_str().len())
        .unwrap_or(path);
    crate::remote::is_detection_tooling(&relative.to_string_lossy().replace('\\', "/"))
}

/// Obfuscation tells, used to qualify a "content after the module end"
/// finding. Without this a flat config that legitimately runs long is flagged.
fn looks_like_payload(tail: &str) -> bool {
    const TELLS: &[&str] = &[
        "eval(",
        "new Function(",
        "Buffer.from(",
        "child_process",
        "atob(",
        "fromCharCode",
    ];
    TELLS.iter().any(|t| tail.contains(t)) || tail.lines().any(|l| l.len() > 500)
}

/// A config whose module ends and is then followed by a long remainder that
/// looks like a payload. Returns the line count and the line the module ends
/// on. Shared by the machine check and the remote one, so the two cannot come
/// to disagree about what a suspicious config looks like.
pub(crate) fn payload_shaped_tail(body: &str) -> Option<(usize, usize)> {
    let lines: Vec<&str> = body.lines().collect();
    let total = lines.len();
    let end = lines
        .iter()
        .rposition(|l| l.starts_with("export default") || l.starts_with("module.exports"))?;
    if total <= end + 16 {
        return None;
    }
    let tail = lines.get(end + 1..).unwrap_or_default().join("\n");
    looks_like_payload(&tail).then_some((total, end + 1))
}

/// `.vscode/tasks.json`, the editor task that runs on folder open.
pub(crate) fn is_tasks_json(p: &Path) -> bool {
    p.file_name().and_then(|n| n.to_str()) == Some("tasks.json")
        && p.parent()
            .and_then(|d| d.file_name())
            .and_then(|n| n.to_str())
            == Some(".vscode")
}

/// The folder-open task check, one file at a time, so a scan can read each
/// `tasks.json` the moment the walk finds it. ADR-0059.
pub struct TasksJson {
    seen: usize,
    flagged: usize,
}

impl TasksJson {
    pub fn open(v: &mut Verdict) -> Self {
        v.section("Workspace tasks that run on folder open");
        Self {
            seen: 0,
            flagged: 0,
        }
    }

    pub fn inspect(&mut self, path: &Path, ind: &Indicators, v: &mut Verdict, sink: &mut Sink) {
        self.seen += 1;
        let Ok(bytes) = fs::read(path) else {
            return;
        };
        // A published campaign tasks.json is confirmed by what it is, even
        // where it carries no indicator string: ADR-0045.
        if let Some(label) = ind.known_file(&bytes) {
            self.flagged += 1;
            let line = sink.take(path, "malicious-tasks-json");
            v.push(
                Finding::hit(
                    Kind::TasksJson,
                    format!(
                        "tasks.json matches a known campaign file ({label}): {}",
                        path.display()
                    ),
                )
                .at(path)
                .with_remedy(line),
            );
            return;
        }
        let body = String::from_utf8_lossy(&bytes);
        if !crate::unescape::contains_decoded(&body, "folderOpen") {
            return;
        }
        self.flagged += 1;
        if ind.has_strong(&body) {
            let line = sink.take(path, "malicious-tasks-json");
            v.push(
                Finding::hit(
                    Kind::TasksJson,
                    format!(
                        "tasks.json runs on folder open AND contains an indicator: {}",
                        path.display()
                    ),
                )
                .at(path)
                .with_remedy(line),
            );
        } else {
            v.push(
                Finding::review(format!(
                    "tasks.json runs on folder open, verify the command by hand: {}",
                    path.display()
                ))
                .at(path),
            );
        }
    }

    pub fn finish(self, v: &mut Verdict) {
        if self.flagged == 0 {
            v.push(Finding::ok(if self.seen == 0 {
                "no .vscode/tasks.json found under the scanned paths".to_string()
            } else {
                format!(
                    "{} .vscode/tasks.json checked, none run on folder open",
                    self.seen
                )
            }));
        }
    }
}

pub fn tasks_json(walk: &Walk, ind: &Indicators, v: &mut Verdict, sink: &mut Sink) {
    let mut check = TasksJson::open(v);
    for path in walk.files.iter().filter(|p| is_tasks_json(p)) {
        check.inspect(path, ind, v, sink);
    }
    check.finish(v);
}

/// What to do with a build config that carries an indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnInfectedConfig {
    /// `check`: report it and leave it. `--apply` moves files, and moving this
    /// one would take the project's own config with it.
    Report,
    /// `clean`: cut an appended payload out of the file, keeping the original.
    Strip,
}

const DO_NOT_EDIT: &str = "do not edit this file by hand. `polinrider clean` strips an appended payload and keeps the original; otherwise delete the clone and re-clone after the remote is clean.";

/// A hit in a file that strip may cut: report it, cut it, or say why not.
/// Returns whether it was cut.
fn infected_source(
    path: &Path,
    message: String,
    planned: Result<Plan, strip::Refusal>,
    on_infected: OnInfectedConfig,
    v: &mut Verdict,
    sink: &mut Sink,
) -> bool {
    let hit = Finding::hit(
        Kind::Config {
            strippable: planned.is_ok(),
        },
        message,
    )
    .at(path);
    match (on_infected, planned) {
        (OnInfectedConfig::Report, _) => {
            v.push(hit.with_remedy(DO_NOT_EDIT));
            false
        }
        (OnInfectedConfig::Strip, Ok(plan)) => {
            show_cut(&plan, v);
            v.push(hit.with_remedy(sink.strip(path, &plan)));
            true
        }
        (OnInfectedConfig::Strip, Err(refusal)) => {
            v.push(hit.with_remedy(format!(
                "not stripped: {}. Delete the clone and re-clone after the remote is clean.",
                refusal.reason()
            )));
            false
        }
    }
}

/// Said every time something was stripped, because it is the part people
/// assume the opposite of: the working tree is clean and the repository is
/// not.
fn strip_notes(v: &mut Verdict) {
    v.push(Finding::info(
        "Stripping cleans the files in the working tree. It does not touch git: the commit that carried the payload may still be in this repository's history and on its remote.",
    ));
    v.push(Finding::info(
        "Review each change with git diff before committing it. Nothing was staged, committed, reset or stashed.",
    ));
}

/// The build-config check, one file at a time. ADR-0059.
pub struct BuildConfigs {
    seen: usize,
    flagged: usize,
    stripped: usize,
    on_infected: OnInfectedConfig,
}

impl BuildConfigs {
    pub fn open(on_infected: OnInfectedConfig, v: &mut Verdict) -> Self {
        v.section("Build configs with code after the module end");
        Self {
            seen: 0,
            flagged: 0,
            stripped: 0,
            on_infected,
        }
    }

    pub fn inspect(&mut self, path: &Path, ind: &Indicators, v: &mut Verdict, sink: &mut Sink) {
        self.seen += 1;
        // Bytes, not a string: a payload does not promise to be valid UTF-8,
        // and a file that failed to decode used to be skipped without a word.
        let Ok(bytes) = fs::read(path) else {
            v.push(
                Finding::review(format!(
                    "could not read, so it was not checked: {}",
                    path.display()
                ))
                .at(path),
            );
            return;
        };
        let body = String::from_utf8_lossy(&bytes);

        if let Some(planned) = strip::plan(&bytes, ind) {
            self.flagged += 1;
            let message = format!("config file contains an indicator: {}", path.display());
            if infected_source(path, message, planned, self.on_infected, v, sink) {
                self.stripped += 1;
            }
            return;
        }

        // A whole file known from the campaign, with no indicator string to
        // cut at: confirmed, and not one strip will touch. ADR-0045.
        if let Some(label) = ind.known_file(&bytes) {
            self.flagged += 1;
            v.push(
                Finding::hit(
                    Kind::Config { strippable: false },
                    format!(
                        "config file matches a known campaign file ({label}): {}",
                        path.display()
                    ),
                )
                .at(path)
                .with_remedy(
                    "not stripped: the whole file is a known campaign file. Delete the clone and re-clone after the remote is clean.",
                ),
            );
            return;
        }

        if let Some((total, end)) = payload_shaped_tail(&body) {
            self.flagged += 1;
            v.push(
                Finding::review(format!(
                    "content after module end that looks like a payload ({total} lines, module ends at {end}): {}",
                    path.display()
                ))
                .at(path),
            );
        }

        if body.lines().any(|l| l.len() > 4000) {
            self.flagged += 1;
            v.push(
                Finding::review(format!(
                    "line longer than 4000 characters, an obfuscation tell: {}",
                    path.display()
                ))
                .at(path),
            );
        }
    }

    /// Close the section. Returns how many configs were stripped.
    pub fn finish(self, v: &mut Verdict) -> usize {
        if self.flagged == 0 {
            v.push(Finding::ok(format!(
                "{} build config files checked, nothing appended after the module end",
                self.seen
            )));
        }
        if self.stripped > 0 {
            strip_notes(v);
        }
        self.stripped
    }
}

/// Every build config in a walk. For tests and for a caller that has the
/// whole walk already.
pub fn build_configs(
    walk: &Walk,
    ind: &Indicators,
    on_infected: OnInfectedConfig,
    v: &mut Verdict,
    sink: &mut Sink,
) -> usize {
    let mut check = BuildConfigs::open(on_infected, v);
    for path in walk.by_name(is_build_config) {
        check.inspect(path, ind, v, sink);
    }
    check.finish(v)
}

/// The project files, each read once: every JavaScript and TypeScript file
/// for a confirmed indicator (ADR-0058), and every file under 1 MiB for the
/// weak signals (ADR-0047): not binary, the first string per file, matched as
/// written and not decoded, the operator's own detection files and
/// documentation set aside as the GitHub check sets them aside. One pass, so a file is read once however many
/// questions are asked of it, and one honest count of files read.
///
/// `configs_stripped` says whether the config check has already said what
/// stripping does and does not do.
pub struct ProjectFiles<'a> {
    pub roots: &'a [PathBuf],
    pub on_infected: OnInfectedConfig,
    pub configs_stripped: bool,
    /// Where a check that continues a stopped one picks the pass up.
    pub resume: Option<&'a crate::progress::Resume>,
}

/// What the pass tells the scan as it goes.
pub enum Pass<'a> {
    /// How many files have been gone through, with the findings made since
    /// the last time: every thousand files or so and after each finding.
    Count(usize, Vec<Finding>),
    /// A save point: the `n`th file is `at`, every file before it is done,
    /// and these are the files the pass found something in so far. Every
    /// [`crate::progress::SAVE_EVERY`] files, after their findings were told.
    /// ADR-0060.
    Save {
        n: usize,
        at: &'a Path,
        found: &'a [PathBuf],
    },
}

/// Files are gone through in the walk's sorted order, so a position in the
/// pass is a path. A check that continues another skips every file before
/// its save point, except those that had a finding, which are read again.
pub fn project_files(
    walk: &Walk,
    files: &ProjectFiles,
    ind: &Indicators,
    v: &mut Verdict,
    sink: &mut Sink,
    tick: &mut dyn FnMut(Pass),
) -> crate::progress::Skipped {
    // A file another check already reported is not read again for a weak
    // signal. Sorted, so a home folder's worth of findings is a lookup.
    let mut reported: Vec<PathBuf> = v.findings().filter_map(|f| f.path.clone()).collect();
    reported.sort();
    let mut sources = Verdict::new();
    sources.section("JavaScript and TypeScript files carrying an indicator");
    let mut weak = Verdict::new();
    weak.section("Weak signals, which legitimate projects also contain");
    let (mut sources_read, mut weak_read, mut stripped) = (0usize, 0usize, 0usize);
    let mut source_hits = 0usize;
    let mut weak_hits = 0usize;
    let mut found: Vec<PathBuf> = Vec::new();
    let mut skipped = crate::progress::Skipped::default();

    for (n, path) in walk.files.iter().enumerate() {
        let before = sources.hits() + weak.reviews();
        let save = n > 0 && n % crate::progress::SAVE_EVERY == 0;
        if n % 1024 == 0 || save {
            pass_tick(tick, n, &mut sources, &mut weak);
        }
        if save {
            tick(Pass::Save {
                n,
                at: path,
                found: &found,
            });
        }
        if let Some(resume) = files.resume {
            if *path < resume.from {
                if resume.reread.binary_search(path).is_ok() {
                    skipped.reread += 1;
                } else {
                    skipped.skipped += 1;
                    continue;
                }
            }
        }
        let name = path.file_name().and_then(|x| x.to_str()).unwrap_or("");
        let tooling = is_tooling(path, files.roots);
        let as_source = is_js_source(name) && !is_build_config(name) && !tooling;
        let as_weak = !ind.weak.is_empty()
            && !tooling
            && reported.binary_search(path).is_err()
            // Editor files: tasks.json has a stricter check of its own, and
            // folderOpen in settings.json is a known false positive the
            // corpus pins (settings-json-is-not-tasks-json).
            && !path.components().any(|c| c.as_os_str() == ".vscode")
            && fs::metadata(path).is_ok_and(|m| m.len() <= 1024 * 1024);
        if !as_source && !as_weak {
            continue;
        }
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        if bytes.iter().take(8192).any(|b| *b == 0) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);

        // The cheap question first, so a clean file is searched once.
        if as_source {
            sources_read += 1;
            if ind.has_strong(&text) {
                if let Some(planned) = strip::plan(&bytes, ind) {
                    source_hits += 1;
                    let message = format!("source file contains an indicator: {}", path.display());
                    if infected_source(
                        path,
                        message,
                        planned,
                        files.on_infected,
                        &mut sources,
                        sink,
                    ) {
                        stripped += 1;
                    }
                    // Reported here, so not again as a weak signal. Told
                    // now, and kept for a check that continues this one.
                    found.push(path.clone());
                    pass_tick(tick, n + 1, &mut sources, &mut weak);
                    continue;
                }
            }
        }
        if as_weak {
            weak_read += 1;
            if let Some(signal) = ind.weak.iter().find(|w| text.contains(w.as_str())) {
                weak_hits += 1;
                weak.push(
                    Finding::review(format!(
                        "mentions something the campaign also uses ({}), verify by hand: {}",
                        crate::verdict::clean(signal),
                        path.display()
                    ))
                    .at(path),
                );
            }
        }
        if sources.hits() + weak.reviews() > before {
            found.push(path.clone());
            pass_tick(tick, n + 1, &mut sources, &mut weak);
        }
    }
    pass_tick(tick, walk.files.len(), &mut sources, &mut weak);
    if source_hits == 0 {
        sources.push(Finding::ok(format!(
            "{sources_read} JavaScript and TypeScript files read, none carries an indicator"
        )));
    }
    if stripped > 0 && !files.configs_stripped {
        strip_notes(&mut sources);
    }
    if ind.weak.is_empty() {
        weak.push(Finding::ok("no weak indicators to look for"));
    } else if weak_hits == 0 {
        weak.push(Finding::ok(format!(
            "{weak_read} files read for weak signals, none matched"
        )));
    }
    // Anything found since the last tick is handed on with the sections.
    v.append(sources);
    v.append(weak);
    skipped
}

/// Hand the pass's new findings to the scan. The
/// findings stay in their own sections until the pass is done; only the
/// stream sees them early.
fn pass_tick(tick: &mut dyn FnMut(Pass), n: usize, sources: &mut Verdict, weak: &mut Verdict) {
    let mut fresh = sources.take_fresh();
    fresh.extend(weak.take_fresh());
    tick(Pass::Count(n, fresh))
}

/// The evidence for a cut, printed above the finding: the last line that
/// stays and the start of what goes. Shown in a dry run too, which is the
/// point of it. Nobody should have to take the cut on trust.
fn show_cut(plan: &Plan, v: &mut Verdict) {
    // The last line as it will be, not as it is: in the file as found, that
    // line still has the padding and the payload on the end of it.
    let kept = String::from_utf8_lossy(&plan.keep);
    v.detail(format!(
        "keeps through line {}: {}",
        plan.last_kept_line,
        kept.lines().last().unwrap_or_default()
    ));
    v.detail(format!(
        "cuts {} bytes, starting: {}",
        plan.removed, plan.preview
    ));
}

pub fn fonts(walk: &Walk, ind: &Indicators, v: &mut Verdict, sink: &mut Sink) {
    v.section("Font files that are not fonts");
    let mut seen = 0usize;
    let mut flagged = 0usize;

    for path in
        walk.by_name(|n| (n.ends_with(".woff") || n.ends_with(".woff2")) && !n.starts_with("._"))
    {
        if path.components().any(|c| c.as_os_str() == "__MACOSX") {
            continue;
        }
        seen += 1;
        let Ok(bytes) = fs::read(path) else { continue };

        // A published fake font, by its hash: named, and caught even if it
        // ever starts with a font's magic bytes. ADR-0045.
        if let Some(label) = ind.known_file(&bytes) {
            flagged += 1;
            let line = sink.take(path, "font-masquerade");
            v.push(
                Finding::hit(
                    Kind::FakeFont,
                    format!(
                        "font file matches a known campaign file ({label}): {}",
                        path.display()
                    ),
                )
                .at(path)
                .with_remedy(line),
            );
            continue;
        }

        // An empty file has no magic bytes to check, and a Git LFS pointer is
        // a text stub standing in for the font. Both are known false
        // positives, both pinned by conformance cases.
        if bytes.is_empty() || bytes.starts_with(b"version https://git-lfs") {
            continue;
        }
        let Some(magic) = bytes.get(..4) else {
            // Shorter than four bytes and not empty: not a font, but not worth
            // a confirmed hit either. The shell skips these too.
            continue;
        };
        if magic == b"wOFF" || magic == b"wOF2" || magic == b"vers" {
            continue;
        }

        flagged += 1;
        let hex = magic
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ");
        let line = sink.take(path, "font-masquerade");
        v.push(
            Finding::hit(
                Kind::FakeFont,
                format!(
                    "font file is not a font (first bytes: {hex}): {}",
                    path.display()
                ),
            )
            .at(path)
            .with_remedy(line),
        );
    }

    if flagged == 0 {
        v.push(Finding::ok(format!(
            "{seen} font files checked, all are real fonts"
        )));
    }
}

/// Campaign packages a project installs, read out of its manifests and
/// lockfiles by name (ADR-0041). A name inside a longer one, or in a
/// description, is not a dependency on it.
pub fn packages(walk: &Walk, ind: &Indicators, v: &mut Verdict) {
    v.section("Known-bad packages");
    let mut found = false;

    for path in walk.by_name(manifest::is_manifest) {
        let Ok(bytes) = fs::read(path) else {
            v.push(Finding::review(format!(
                "could not read, so it was not checked: {}",
                path.display()
            )));
            continue;
        };
        let body = String::from_utf8_lossy(&bytes);
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let hit = || {
            Finding::hit(
                Kind::Package,
                format!("known-bad package referenced: {}", path.display()),
            )
            .at(path)
            .with_remedy(
                "remove the dependency, delete node_modules and the lockfile entry, reinstall.",
            )
        };
        match ind.manifest_packages(name, &body) {
            ManifestMatch::Clean => {}
            ManifestMatch::Names(names) => {
                found = true;
                for package in &names {
                    v.detail(package);
                }
                v.push(hit());
            }
            ManifestMatch::Unreadable { text_match, .. } => {
                // Never silently clean: the text is matched as 1.x matched
                // it, and the reader is told why.
                if text_match {
                    found = true;
                    v.push(hit());
                }
                v.push(Finding::review(format!(
                    "could not be read as a manifest, so package names were matched as text: {}",
                    path.display()
                )));
            }
        }
    }

    if !found {
        v.push(Finding::ok(
            "no known-bad package names in manifests or lockfiles",
        ));
    }
}

/// The git hook check, one `.git` directory at a time, so a scan can read
/// each repository's hooks the moment the walk finds it. ADR-0059.
pub struct GitHooks;

impl GitHooks {
    pub fn open(host: Option<&dyn Host>, v: &mut Verdict) -> Self {
        v.section("Git configuration and hooks");
        match host {
            Some(host) => host_checks::git_global_config(host, v),
            // The global hooksPath lives in the user's git config, which is
            // host state; under --fs-only it is not read. Reported as ok so
            // the section always says something rather than printing nothing.
            None => v.push(Finding::ok("no global core.hooksPath")),
        }
        Self
    }

    pub fn inspect(&mut self, git_dir: &Path, ind: &Indicators, v: &mut Verdict, sink: &mut Sink) {
        for hook in crate::walk::hooks_in(git_dir) {
            if ind.file_has_strong(&hook) {
                let line = sink.take(&hook, "git-hook");
                v.push(
                    Finding::hit(
                        Kind::GitHook,
                        format!("git hook contains an indicator: {}", hook.display()),
                    )
                    .at(&hook)
                    .with_remedy(line),
                );
            } else {
                v.push(
                    Finding::review(format!(
                        "active git hook, verify by hand: {}",
                        hook.display()
                    ))
                    .at(&hook),
                );
            }
        }
    }
}

pub fn git_hooks(
    walk: &Walk,
    ind: &Indicators,
    host: Option<&dyn Host>,
    v: &mut Verdict,
    sink: &mut Sink,
) {
    let mut check = GitHooks::open(host, v);
    for git_dir in &walk.git_dirs {
        check.inspect(git_dir, ind, v, sink);
    }
}

/// The second-stage implant: known install and persistence paths, plus a hash
/// sweep so a renamed binary is still caught.
///
/// `~` in `implant-paths.txt` expands against the home directory given here,
/// not the process environment, so a scan of a mounted backup can point at the
/// backup's home rather than the running user's.
/// `%LOCALAPPDATA%\X` as a path under a home directory. `None` for a
/// variable that does not live under one.
fn windows_path(home: &Path, line: &str) -> Option<PathBuf> {
    let rest = line.strip_prefix('%')?;
    let (variable, tail) = rest.split_once('%')?;
    let base = match variable.to_ascii_uppercase().as_str() {
        "USERPROFILE" => home.to_path_buf(),
        "LOCALAPPDATA" => home.join("AppData").join("Local"),
        "APPDATA" => home.join("AppData").join("Roaming"),
        "TEMP" | "TMP" => home.join("AppData").join("Local").join("Temp"),
        _ => return None,
    };
    Some(
        tail.split(['\\', '/'])
            .filter(|part| !part.is_empty())
            .fold(base, |path, part| path.join(part)),
    )
}

/// What to run before quarantining a file that is how the implant starts
/// itself. Read from where the file is: a launch agent, a systemd user unit
/// or an autostart entry.
fn stop_first(path: &Path) -> Vec<String> {
    let text = path.display().to_string();
    let quoted = sh_quote(&text);
    if text.contains("/LaunchAgents/") {
        vec![format!(
            "stop it first: launchctl bootout gui/$(id -u) {quoted} 2>/dev/null || launchctl unload {quoted}"
        )]
    } else if text.contains("/systemd/user/") {
        let unit = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        vec![
            format!(
                "stop it first: systemctl --user disable --now {}",
                sh_quote(&unit)
            ),
            "and: loginctl disable-linger \"$(whoami)\"".to_string(),
        ]
    } else if text.contains("/autostart/") {
        vec!["it will not start again once this file is quarantined".to_string()]
    } else {
        Vec::new()
    }
}

/// Where the implant check looks beyond the files of the walk.
pub struct ImplantScope<'a> {
    /// The home directory its install paths are under, when there is one.
    pub home: Option<&'a Path>,
    pub ioc_dir: &'a Path,
    /// The machine, for its process table.
    pub host: Option<&'a dyn Host>,
}

/// The second-stage implant where it installs itself, and in the process
/// table. Step 1: it runs by itself. The renamed copy anywhere on the disk is
/// [`known_hashes`], step 3.
pub fn implant_installs(ind: &Indicators, scope: &ImplantScope, v: &mut Verdict, sink: &mut Sink) {
    v.section("Second-stage implant");
    let (home, ioc_dir, host) = (scope.home, scope.ioc_dir, scope.host);
    let mut found = false;

    // The paths the implant installs itself to. They are under the home
    // directory, so `clean`, which is pointed at a repository and nothing
    // else, passes no home and this half does not run.
    let paths = home.and_then(|_| fs::read_to_string(ioc_dir.join("implant-paths.txt")).ok());
    if let (Some(home), Some(text)) = (home, paths) {
        for line in text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let path = if line.starts_with('%') {
                // A Windows path. Read against the home directory given, so
                // that a Windows profile on a backup drive is checked too.
                match windows_path(home, line) {
                    Some(path) => path,
                    None => continue,
                }
            } else {
                match line.strip_prefix("~/") {
                    Some(rest) => home.join(rest),
                    // A path from the root of a Unix system means nothing on
                    // Windows, where it would be read against the current drive.
                    None if cfg!(windows) => continue,
                    None => Path::new(line).to_path_buf(),
                }
            };
            if !path.exists() {
                continue;
            }
            found = true;
            let line_out = sink.take(&path, "second-stage-implant");
            let mut finding = Finding::hit(
                Kind::Implant,
                format!("implant artifact present: {}", path.display()),
            )
            .at(&path);
            // Moving the file does not stop what it already started. Said
            // first, because it has to be done first.
            for advice in stop_first(&path) {
                finding = finding.with_remedy(advice);
            }
            v.push(finding.with_remedy(line_out));
        }
    }

    // The process table. The implant sets its own process title, so a match
    // there is a finding even with nothing on disk.
    let Some(host) = host else {
        if !found {
            v.push(Finding::ok(
                "no second-stage implant at its install paths (--fs-only: process table not read)",
            ));
        }
        return;
    };
    match host_checks::implant_processes(host, ind, v) {
        ProcessCheck::Running => {}
        ProcessCheck::NoneRunning if !found => {
            v.push(Finding::ok(
                "no second-stage implant at its install paths, and none running",
            ));
        }
        // Not read: the review line above already says so, and "none found"
        // may only be claimed for the half that was looked at.
        ProcessCheck::NotRead if !found => {
            v.push(Finding::ok("no second-stage implant at its install paths"));
        }
        ProcessCheck::NoneRunning | ProcessCheck::NotRead => {}
    }
}

/// The smallest and largest file the hash sweep reads, and the reason it
/// exists: the MicrosoftSystem64 binaries, 67 to 87 MB.
const SWEEP: std::ops::RangeInclusive<u64> = 10 * 1024 * 1024..=300 * 1024 * 1024;

/// Does this file start like a program: ELF, Mach-O (thin in either byte
/// order, or fat) or a Windows PE (`MZ`)? Every known hash the sweep is for is
/// one of these, and equal hashes mean equal bytes, so a file that starts any
/// other way cannot match one. ADR-0059.
pub(crate) fn starts_like_program(first: &[u8]) -> bool {
    const MAGIC: &[&[u8]] = &[
        b"\x7fELF",
        // Mach-O 32 and 64 bit, as written and byte-swapped.
        b"\xfe\xed\xfa\xce",
        b"\xfe\xed\xfa\xcf",
        b"\xce\xfa\xed\xfe",
        b"\xcf\xfa\xed\xfe",
        // Universal (fat) Mach-O, 32 and 64 bit headers, both orders. Java
        // class files share the first; they are hashed too, which costs time
        // and loses nothing.
        b"\xca\xfe\xba\xbe",
        b"\xbe\xba\xfe\xca",
        b"\xca\xfe\xba\xbf",
        b"\xbf\xba\xfe\xca",
        b"MZ",
    ];
    MAGIC.iter().any(|m| first.starts_with(m))
}

fn first_bytes(path: &Path) -> Option<[u8; 4]> {
    use std::io::Read;
    let mut head = [0u8; 4];
    let mut file = fs::File::open(path).ok()?;
    let mut got = 0;
    while got < head.len() {
        match file.read(head.get_mut(got..)?) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    Some(head)
}

/// A renamed implant still hashes the same. Every file of 10 MB to 300 MB
/// that starts like a program is hashed and compared; the rest are passed
/// over unread past their first four bytes. Step 3.
///
/// `hashing` is told (done, of) before each file is hashed, with the findings
/// made since.
pub fn known_hashes(
    walk: &Walk,
    ind: &Indicators,
    v: &mut Verdict,
    sink: &mut Sink,
    hashing: &mut dyn FnMut(usize, usize, Vec<Finding>),
) {
    v.section("Large files against known hashes");
    let known = &ind.hashes;
    if known.is_empty() {
        v.push(Finding::ok("no known hashes to compare"));
        return;
    }
    let large: Vec<&PathBuf> = walk
        .files
        .iter()
        .filter(|path| fs::metadata(path).is_ok_and(|m| SWEEP.contains(&m.len())))
        .collect();
    let programs: Vec<&PathBuf> = large
        .iter()
        .copied()
        .filter(|path| first_bytes(path).is_some_and(|head| starts_like_program(&head)))
        .collect();
    let mut found = false;
    for (n, path) in programs.iter().copied().enumerate() {
        hashing(n, programs.len(), v.take_fresh());
        let Ok(digest) = crate::sha256::file(path) else {
            continue;
        };
        if let Some(label) = known.iter().find(|(h, _)| *h == digest).map(|(_, l)| l) {
            found = true;
            let line_out = sink.take(path, "second-stage-implant");
            v.push(
                Finding::hit(
                    Kind::Implant,
                    format!(
                        "file matches a known implant hash ({label}): {}",
                        path.display()
                    ),
                )
                .at(path)
                .with_remedy(line_out),
            );
        }
    }
    hashing(programs.len(), programs.len(), v.take_fresh());
    if !found {
        v.push(Finding::ok(
            "no file under the scanned paths matches a known implant hash",
        ));
    }
    v.note(format!(
        "Hashed {} of the {} files of 10 MB to 300 MB: the ones that start like a program. Every known hash of that size is a program, so the others cannot match.",
        programs.len(),
        large.len()
    ));
}

/// The propagation script the campaign drops to spread to other remotes.
pub fn propagation(walk: &Walk, v: &mut Verdict, sink: &mut Sink) {
    v.section("Propagation artifact temp_auto_push.bat");
    let mut found = false;
    for path in walk.by_name(is_propagation_script) {
        found = true;
        let line = sink.take(path, "propagation-script");
        v.push(
            Finding::hit(
                Kind::Propagation,
                format!("propagation script present: {}", path.display()),
            )
            .at(path)
            .with_remedy(line),
        );
    }
    if !found {
        v.push(Finding::ok("temp_auto_push.bat not found"));
    }
}

fn is_propagation_script(name: &str) -> bool {
    name == "temp_auto_push.bat" || name == "config.bat"
}

/// Files whose name alone is an indicator in `ioc/filenames.txt`, such as a
/// fake font renamed to `.llf`. 1.x matched these names in a path scan and in
/// a GitHub scan; `remote.rs` carries the GitHub side and this the path side.
/// A file check, so it runs with `--fs-only` too. The propagation script is
/// left to its own check, which says what it is.
pub fn named_files(walk: &Walk, ind: &Indicators, v: &mut Verdict, sink: &mut Sink) {
    v.section("Files named as indicators");
    // A file another check already reported, the implant at its install path
    // for one, is reported once and quarantined once.
    let reported: Vec<PathBuf> = v.findings().filter_map(|f| f.path.clone()).collect();
    let mut named = false;
    for path in &walk.files {
        if reported.iter().any(|p| p == path) {
            continue;
        }
        let is_script_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(is_propagation_script);
        // Matched on the path with forward slashes, as the patterns are
        // written, so a Windows path is read the same way.
        let text = path.to_string_lossy().replace('\\', "/");
        if is_script_name {
            continue;
        }
        if !ind.is_bad_filename(&text) {
            // A name legitimate projects can have too: a look, not a verdict.
            // ADR-0048.
            if ind.is_review_filename(&text) {
                let name = path.file_name().map_or_else(
                    || text.clone(),
                    |n| crate::verdict::clean(&n.to_string_lossy()),
                );
                v.push(
                    Finding::review(format!(
                        "file name the campaign also uses ({name}), verify by hand: {}",
                        path.display()
                    ))
                    .at(path),
                );
            }
            continue;
        }
        named = true;
        let line = sink.take(path, "named-file");
        v.push(
            Finding::hit(
                Kind::NamedFile,
                format!("file named as an indicator: {}", path.display()),
            )
            .at(path)
            .with_remedy(line),
        );
    }
    if !named {
        v.push(Finding::ok("no file named as an indicator"));
    }
}

/// Editor extensions, which is how the campaign most often arrives.
///
/// Scoped to the extension directories given, and matched on content. The
/// weak list is deliberately not used here: a bundled extension legitimately
/// contains "folderOpen", which is a codicon name, and matching it produces
/// pages of noise.
pub fn extensions(dirs: &[PathBuf], ind: &Indicators, v: &mut Verdict, sink: &mut Sink) {
    v.section("IDE extensions");
    let mut any_dir = false;
    let mut found = false;

    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        any_dir = true;
        // Nothing is pruned here. An extension ships its dependencies inside
        // its own node_modules, and that is as good a place for a payload as
        // any. The project walk prunes node_modules because a project's
        // dependencies are caught by name from its lockfile; an installed
        // extension has no lockfile to catch them from.
        let everything = crate::walk::Options {
            prune: &[],
            skip: None,
            skip_file: None,
        };
        let w = crate::walk::walk_with(std::slice::from_ref(dir), &everything);
        let mut flagged: Vec<PathBuf> = Vec::new();
        // Files that name a campaign host or address and carry no confirmed
        // indicator, with what they name.
        let mut naming: Vec<(&Path, Vec<&str>)> = Vec::new();

        for file in w.by_name(|n| {
            [".js", ".mjs", ".cjs", ".ts", ".json", ".map"]
                .iter()
                .any(|e| n.ends_with(e))
        }) {
            let Ok(bytes) = fs::read(file) else {
                continue;
            };
            let text = String::from_utf8_lossy(&bytes);
            if !ind.has_strong(&text) {
                // The weak list is useless in an extension bundle: one
                // legitimately contains "folderOpen" and "windowsHide". Only
                // the campaign's own infrastructure is worth a human's time.
                let named = ind.infrastructure_named(&text);
                if !named.is_empty() {
                    naming.push((file, named));
                }
                continue;
            }
            // Report the extension, not every file inside it.
            let ext_root = file
                .strip_prefix(dir)
                .ok()
                .and_then(|r| r.components().next())
                .map(|c| dir.join(c.as_os_str()));
            if let Some(root) = ext_root {
                if !flagged.contains(&root) {
                    flagged.push(root);
                }
            }
        }

        // An extension already confirmed is not also listed for review.
        naming.retain(|(file, _)| !flagged.iter().any(|root| file.starts_with(root)));
        for (file, named) in naming.iter().take(20) {
            v.push(
                Finding::review(format!(
                    "extension references campaign infrastructure ({}): {}",
                    named.join(" "),
                    file.display()
                ))
                .at(file),
            );
        }

        for root in flagged {
            found = true;
            let line = sink.take(&root, "ide-extension");
            v.push(
                Finding::hit(
                    Kind::Extension,
                    format!("extension contains an indicator: {}", root.display()),
                )
                .at(&root)
                .with_remedy(line),
            );
        }
    }

    if !any_dir {
        v.push(Finding::ok("no IDE extension directories found"));
        return;
    }
    if !found {
        v.push(Finding::ok("no IDE extension contains an indicator"));
    }

    // What changed lately, by name, in the report. Inventory and not a
    // finding: an extension updates itself, and the one to worry about is the
    // one nobody remembers installing.
    const RECENT_DAYS: u64 = 60;
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(RECENT_DAYS * 86_400));
    let mut recent = 0usize;
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter(|e| {
                let changed = e.metadata().and_then(|m| m.modified()).ok();
                matches!((changed, cutoff), (Some(changed), Some(cutoff)) if changed >= cutoff)
            })
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort_unstable();
        recent += names.len();
        for name in names {
            v.note(format!("    {name}"));
        }
    }
    if recent > 0 {
        v.push(Finding::info(format!(
            "{recent} extension{} installed or updated in the last {RECENT_DAYS} days. Names are in the report; check any you did not install yourself.",
            if recent == 1 { "" } else { "s" }
        )));
    }
}

/// SSH keys the campaign adds to `~/.ssh/authorized_keys` so that it can log
/// in after the implant is gone. Each key is matched on its own fields, the
/// comment or the key itself, never on the whole line, which also holds
/// options. The file holds the owner's own keys, so it is never moved or
/// edited: the line is named and removed by hand. ADR-0049.
pub fn ssh_keys(home: &Path, ind: &Indicators, v: &mut Verdict) {
    v.section("SSH keys that let someone in");
    let path = home.join(".ssh").join("authorized_keys");
    let text = match fs::read(&path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            v.push(Finding::ok("no ~/.ssh/authorized_keys"));
            return;
        }
        Err(e) => {
            v.push(Finding::review(format!(
                "could not read, so it was not checked: {} ({e})",
                path.display()
            )));
            return;
        }
    };
    let is_key_type =
        |f: &&str| f.starts_with("ssh-") || f.starts_with("ecdsa-") || f.starts_with("sk-");
    let (mut keys, mut found) = (0usize, false);
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        let Some(at) = fields.iter().position(is_key_type) else {
            continue;
        };
        keys += 1;
        let key = fields.get(at + 1).copied().unwrap_or_default();
        let comment = fields
            .get(at + 2..)
            .map(|c| c.join(" "))
            .unwrap_or_default();
        if let Some(known) = ind.ssh_keys.iter().find(|k| **k == comment || *k == key) {
            found = true;
            v.push(
                Finding::hit(
                    Kind::SshKey,
                    format!(
                        "an SSH key known from the campaign is allowed to log in ({}): {}, line {}",
                        crate::verdict::clean(known),
                        path.display(),
                        n + 1
                    ),
                )
                .at(&path)
                .with_remedy(
                    "Remove that line by hand. It lets the attacker log in over SSH even after the implant is gone.",
                ),
            );
        }
    }
    if !found {
        v.push(Finding::ok(format!(
            "{}: {}, none known from the campaign",
            path.display(),
            if keys == 1 {
                "1 key".to_string()
            } else {
                format!("{keys} keys")
            }
        )));
    }
}

/// What would have to be changed if anything else here is a confirmed
/// finding: private keys and credential files in the home directory, and
/// `.env` files under the scanned paths.
///
/// Inventory, never a finding. Owning an SSH key is not suspicious and a
/// `.pub` file is not a credential. Only paths are listed, and only in the
/// report: nothing is read, so nothing can be printed.
pub fn credentials(walk: &Walk, home: Option<&Path>, v: &mut Verdict) {
    v.section(if home.is_some() {
        "Credential surface on this machine"
    } else {
        "Credential surface under the scanned paths"
    });
    let mut files: Vec<PathBuf> = Vec::new();
    if let Some(home) = home {
        if let Ok(entries) = fs::read_dir(home.join(".ssh")) {
            let mut keys: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("id_") && !n.ends_with(".pub"))
                })
                .collect();
            keys.sort();
            files.extend(keys);
        }
        files.extend(
            [
                ".aws/credentials",
                ".config/gcloud/credentials.db",
                ".docker/config.json",
                ".kube/config",
                ".netrc",
                "_netrc",
            ]
            .iter()
            .map(|p| home.join(p))
            .filter(|p| p.exists()),
        );
    }
    let env: Vec<&PathBuf> = walk.by_name(|n| n.starts_with(".env")).collect();
    for file in files.iter().chain(env.iter().copied()) {
        v.note(format!("  credential material: {}", file.display()));
    }
    if files.is_empty() && env.is_empty() {
        v.push(Finding::ok(
            "no credential files found under the scanned paths",
        ));
        return;
    }
    let count =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    v.push(Finding::info(format!(
        "{} and {} under the scanned paths.",
        count(
            files.len(),
            "private key or credential file",
            "private key or credential files"
        ),
        count(env.len(), ".env file", ".env files")
    )));
    v.push(Finding::info(
        "None of this is a finding. It is the list to change if anything else was a HIT.",
    ));
    v.push(Finding::info("Full paths are in the report file."));
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn a_payload_in_an_extension_s_own_node_modules_is_found() {
        // The shell greps the whole extension directory. The first Rust
        // version reused the project walk, which never enters node_modules,
        // and so could not see a payload in an extension's bundled dependency.
        let dir = std::env::temp_dir().join(format!("prc-ext-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let exts = dir.join("extensions");
        let dep = exts.join("publisher.helper-1.0.0/node_modules/dep");
        fs::create_dir_all(&dep).expect("mkdir");
        fs::write(dep.join("index.js"), "module.exports = 'MARKER-ALPHA'\n").expect("write");
        fs::create_dir_all(exts.join("publisher.fine-2.0.0")).expect("mkdir");
        fs::write(
            exts.join("publisher.fine-2.0.0/extension.js"),
            "exports.activate = () => {}\n",
        )
        .expect("write");

        let ind = Indicators {
            strong: vec!["MARKER-ALPHA".into()],
            ..Indicators::default()
        };
        let q = Quarantine::<DryRun>::new(dir.join("q"));
        let mut v = Verdict::new();
        extensions(&[exts], &ind, &mut v, &mut Sink::Dry(&q));

        let hits: Vec<&str> = v
            .findings()
            .filter(|f| f.level == crate::verdict::Level::Hit)
            .map(|f| f.message.as_str())
            .collect();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(
            hits[0].ends_with("publisher.helper-1.0.0"),
            "the extension, not the file: {hits:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("prc-checks-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn put(path: &Path, body: &str) {
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, body).expect("write");
    }

    #[test]
    fn a_small_file_known_by_its_hash_is_confirmed_in_each_check_that_reads_it() {
        // Harmless bytes stand in for the published files: what is under
        // test is that a check which reads a file compares its hash, at any
        // size. Nothing here is an indicator string.
        let dir = scratch("known-hash");
        let tasks = dir.join("repo/.vscode/tasks.json");
        let font = dir.join("repo/public/fonts/fa-solid-400.woff2");
        let config = dir.join("repo/postcss.config.mjs");
        let other_tasks = dir.join("other/.vscode/tasks.json");
        put(&tasks, "{\"version\":\"2.0.0\",\"tasks\":[]}\n");
        // Starts like a real font, so only the hash can catch it.
        put(&font, "wOF2 not really a font\n");
        put(&config, "export default { plugins: {} }\n");
        // Same name, different bytes: not known, and no folderOpen, so silent.
        put(&other_tasks, "{\"version\":\"2.0.0\"}\n");
        let known = |path: &Path, label: &str| {
            (crate::sha256::file(path).expect("hash"), label.to_string())
        };
        let ind = Indicators {
            strong: vec!["MARKER-ALPHA".into()],
            hashes: vec![
                known(&tasks, "tasks.json fixture"),
                known(&font, "font fixture"),
                known(&config, "config fixture"),
            ],
            ..Indicators::default()
        };
        let walk = Walk {
            files: vec![tasks.clone(), font.clone(), config.clone(), other_tasks],
            ..Walk::default()
        };
        let q = Quarantine::<DryRun>::new(dir.join("q"));
        let mut v = Verdict::new();
        tasks_json(&walk, &ind, &mut v, &mut Sink::Dry(&q));
        fonts(&walk, &ind, &mut v, &mut Sink::Dry(&q));
        build_configs(
            &walk,
            &ind,
            OnInfectedConfig::Strip,
            &mut v,
            &mut Sink::Dry(&q),
        );

        let hits: Vec<(Option<Kind>, &str)> = v
            .findings()
            .filter(|f| f.level == crate::verdict::Level::Hit)
            .map(|f| (f.kind, f.message.as_str()))
            .collect();
        assert_eq!(hits.len(), 3, "{hits:?}");
        assert!(hits.iter().any(|(k, m)| *k == Some(Kind::TasksJson)
            && m.starts_with("tasks.json matches a known campaign file (tasks.json fixture): ")));
        assert!(hits.iter().any(|(k, m)| *k == Some(Kind::FakeFont)
            && m.starts_with("font file matches a known campaign file (font fixture): ")));
        assert!(hits
            .iter()
            .any(|(k, m)| *k == Some(Kind::Config { strippable: false })
                && m.starts_with("config file matches a known campaign file (config fixture): ")));
        // None of the three is a sign that the payload ran on this machine.
        assert!(hits.iter().all(|(k, _)| !k.is_some_and(Kind::ran_here)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_extension_naming_campaign_infrastructure_is_review_and_names_what_matched() {
        let dir = scratch("ext-net");
        let exts = dir.join("extensions");
        put(
            &exts.join("pub.caller-1.0.0/out/main.js"),
            "fetch('http://198.51.100.7/a'); fetch('https://c2.example.test/b')\n",
        );
        // An address that only contains the campaign's is somebody else's.
        put(
            &exts.join("pub.neighbour-1.0.0/out/main.js"),
            "fetch('http://198.51.100.70/a')\n",
        );
        // Confirmed for another reason: reported once, as the hit.
        put(
            &exts.join("pub.both-1.0.0/out/main.js"),
            "var a='MARKER-ALPHA'; fetch('http://198.51.100.7/a')\n",
        );
        let ind = Indicators {
            strong: vec!["MARKER-ALPHA".into()],
            network: vec!["198.51.100.7".into(), "c2.example.test".into()],
            ..Indicators::default()
        };
        let q = Quarantine::<DryRun>::new(dir.join("q"));
        let mut v = Verdict::new();
        extensions(&[exts], &ind, &mut v, &mut Sink::Dry(&q));

        let reviews: Vec<&str> = v
            .findings()
            .filter(|f| f.level == crate::verdict::Level::Review)
            .map(|f| f.message.as_str())
            .collect();
        assert_eq!(reviews.len(), 1, "{reviews:?}");
        assert!(reviews[0].starts_with(
            "extension references campaign infrastructure (198.51.100.7 c2.example.test): "
        ));
        assert!(reviews[0].ends_with("pub.caller-1.0.0/out/main.js"));
        assert_eq!(v.hits(), 1);
        // Fresh on disk, so all three are recent: counted, and named only in
        // the report.
        assert!(v
            .findings()
            .any(|f| f.message.starts_with("3 extensions installed or updated")));
        assert!(v.entries().iter().any(
            |e| matches!(e, crate::verdict::Entry::Note(n) if n.trim() == "pub.caller-1.0.0")
        ));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_implant_that_starts_itself_comes_with_how_to_stop_it_first() {
        let dir = scratch("stop-first");
        let home = dir.join("home");
        let ioc = dir.join("ioc");
        put(
            &ioc.join("implant-paths.txt"),
            "# comment\n~/.config/systemd/user/helper's.service\n~/.config/autostart/helper.desktop\n~/.local/share/helper\n",
        );
        put(
            &home.join(".config/systemd/user/helper's.service"),
            "[Unit]\n",
        );
        put(
            &home.join(".config/autostart/helper.desktop"),
            "[Desktop Entry]\n",
        );
        put(&home.join(".local/share/helper"), "binary\n");
        let q = Quarantine::<DryRun>::new(dir.join("q"));
        let mut v = Verdict::new();
        implant_installs(
            &Indicators::default(),
            &ImplantScope {
                home: Some(&home),
                ioc_dir: &ioc,
                host: None,
            },
            &mut v,
            &mut Sink::Dry(&q),
        );
        let remedies: Vec<String> = v.findings().filter_map(|f| f.remedy.clone()).collect();
        assert_eq!(v.hits(), 3);
        // The unit: stopped by name, quoted, and before the move.
        let unit = remedies
            .iter()
            .find(|r| r.contains("systemctl"))
            .expect("unit advice");
        let lines: Vec<&str> = unit.lines().collect();
        assert_eq!(
            lines[0],
            "stop it first: systemctl --user disable --now 'helper'\\''s.service'"
        );
        assert_eq!(lines[1], "and: loginctl disable-linger \"$(whoami)\"");
        assert_eq!(lines.len(), 3, "the quarantine line comes last: {unit}");
        assert!(remedies
            .iter()
            .any(|r| r.starts_with("it will not start again once this file is quarantined")));
        // A plain file needs no stopping.
        assert!(remedies.iter().any(|r| r.lines().count() == 1));

        let agent = stop_first(Path::new("/Users/x/Library/LaunchAgents/com.helper.plist"));
        assert_eq!(
            agent,
            vec!["stop it first: launchctl bootout gui/$(id -u) '/Users/x/Library/LaunchAgents/com.helper.plist' 2>/dev/null || launchctl unload '/Users/x/Library/LaunchAgents/com.helper.plist'"]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_windows_implant_path_is_read_under_the_home_directory_given() {
        let home = Path::new("/mnt/backup/Users/x");
        assert_eq!(
            windows_path(home, "%LOCALAPPDATA%\\Helper\\helper.exe"),
            Some(home.join("AppData/Local/Helper/helper.exe"))
        );
        assert_eq!(
            windows_path(home, "%AppData%\\Helper"),
            Some(home.join("AppData/Roaming/Helper"))
        );
        assert_eq!(
            windows_path(home, "%USERPROFILE%\\.x"),
            Some(home.join(".x"))
        );
        // Not under a home directory: not guessed at.
        assert_eq!(windows_path(home, "%PROGRAMDATA%\\Helper"), None);
        assert_eq!(windows_path(home, "%broken"), None);
    }

    #[test]
    fn credentials_are_counted_and_named_in_the_report_and_never_read() {
        let dir = scratch("credentials");
        let home = dir.join("home");
        put(&home.join(".ssh/id_ed25519"), "PRIVATE\n");
        put(&home.join(".ssh/id_ed25519.pub"), "public\n");
        put(&home.join(".ssh/known_hosts"), "host\n");
        put(&home.join(".aws/credentials"), "[default]\n");
        put(&home.join("code/app/.env"), "TOKEN=SECRET\n");
        put(&home.join("code/app/.env.local"), "TOKEN=SECRET\n");
        put(&home.join("code/app/env.js"), "export {}\n");
        let w = crate::walk::walk(&[home.join("code")]);

        let mut v = Verdict::new();
        credentials(&w, Some(&home), &mut v);
        let said: Vec<&str> = v.findings().map(|f| f.message.as_str()).collect();
        assert_eq!(
            said[0],
            "2 private key or credential files and 2 .env files under the scanned paths."
        );
        assert!(v.findings().all(|f| f.level == crate::verdict::Level::Info));
        let notes: Vec<&str> = v
            .entries()
            .iter()
            .filter_map(|e| match e {
                crate::verdict::Entry::Note(n) => Some(n.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(notes.len(), 4);
        assert!(notes[0].ends_with(".ssh/id_ed25519"));
        assert!(!notes
            .iter()
            .any(|n| n.ends_with(".pub") || n.contains("known_hosts")));
        assert!(!format!("{:?}", v.entries()).contains("SECRET"));

        // Without a machine to read, only what is under the scanned paths.
        let mut v = Verdict::new();
        credentials(&w, None, &mut v);
        assert_eq!(
            v.findings().next().map(|f| f.message.as_str()),
            Some("0 private key or credential files and 2 .env files under the scanned paths.")
        );
        let mut v = Verdict::new();
        credentials(&Walk::default(), None, &mut v);
        assert_eq!(
            v.findings().next().map(|f| f.level),
            Some(crate::verdict::Level::Ok)
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_config_names_match_the_shell_glob() {
        assert!(is_build_config("postcss.config.mjs"));
        assert!(is_build_config("tailwind.config.js"));
        assert!(is_build_config("next.config.ts"));
        assert!(is_build_config("truffle.js"));
        // Not a config: a source file that merely starts with the same word.
        assert!(!is_build_config("postcssdoesnotcount.js"));
        assert!(!is_build_config("index.js"));
    }

    #[test]
    fn a_payload_in_any_javascript_source_is_confirmed_and_tooling_is_not() {
        // ADR-0058: the local check reads every .js/.ts file for a confirmed
        // indicator, as the GitHub check does, and skips detection tooling
        // judged from the scanned root, not from the absolute path.
        let dir = scratch("js-source");
        let root = dir.join("docs/code");
        let pad = " ".repeat(280);
        let route = root.join("proj/routes/admin.js");
        let jest = root.join("proj/jest.config.js");
        let clean = root.join("proj/src/index.ts");
        let tooling = root.join("proj/docs/detect.js");
        put(
            &route,
            &format!("module.exports = router;{pad}var x='MARKER-ALPHA';\n"),
        );
        put(
            &jest,
            &format!("module.exports = {{}};{pad}var x='MARKER-ALPHA';\n"),
        );
        put(&clean, "export const a = 1\n");
        put(&tooling, "const SIGNS = ['MARKER-ALPHA'];\n");
        let ind = Indicators {
            strong: vec!["MARKER-ALPHA".into()],
            ..Indicators::default()
        };
        let walk = Walk {
            files: vec![route.clone(), jest.clone(), clean, tooling],
            ..Walk::default()
        };
        let q = Quarantine::<DryRun>::new(dir.join("q"));
        let mut v = Verdict::new();
        // Each finding reaches the watcher as it is made, not at the end.
        let mut streamed: Vec<String> = Vec::new();
        project_files(
            &walk,
            &ProjectFiles {
                roots: std::slice::from_ref(&root),
                on_infected: OnInfectedConfig::Strip,
                configs_stripped: false,
                resume: None,
            },
            &ind,
            &mut v,
            &mut Sink::Dry(&q),
            &mut |pass| {
                if let Pass::Count(_, fresh) = pass {
                    streamed.extend(fresh.into_iter().map(|f| f.message));
                }
            },
        );
        assert_eq!(streamed.len(), 2, "{streamed:?}");
        assert!(v.take_fresh().is_empty(), "streamed once, not twice");
        let hits: Vec<(&str, Option<Kind>)> = v
            .findings()
            .filter(|f| f.level == crate::verdict::Level::Hit)
            .map(|f| (f.message.as_str(), f.kind))
            .collect();
        assert_eq!(
            hits,
            vec![
                (
                    format!("source file contains an indicator: {}", route.display()).as_str(),
                    Some(Kind::Config { strippable: true })
                ),
                (
                    format!("source file contains an indicator: {}", jest.display()).as_str(),
                    Some(Kind::Config { strippable: true })
                ),
            ]
        );
        assert!(is_js_source("App.JSX") && is_js_source("server.ts") && !is_js_source("app.json"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_pass_saves_its_place_and_a_continued_one_skips_what_was_read() {
        // Paths only: a file that cannot be read is passed over, so the
        // positions and the save points are all this tests.
        let root = PathBuf::from("/nonexistent-polinrider-pass");
        let files: Vec<PathBuf> = (0..12_000)
            .map(|i| root.join(format!("{i:05}.js")))
            .collect();
        let walk = Walk {
            files: files.clone(),
            ..Walk::default()
        };
        let ind =
            Indicators::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ioc")).expect("ioc");
        let q = Quarantine::<DryRun>::new(root.join("q"));
        let run = |resume: Option<&crate::progress::Resume>| {
            let mut saves = Vec::new();
            let mut v = Verdict::new();
            let skipped = project_files(
                &walk,
                &ProjectFiles {
                    roots: std::slice::from_ref(&root),
                    on_infected: OnInfectedConfig::Report,
                    configs_stripped: false,
                    resume,
                },
                &ind,
                &mut v,
                &mut Sink::Dry(&q),
                &mut |pass| {
                    if let Pass::Save { n, at, .. } = pass {
                        saves.push((n, at.to_path_buf()));
                    }
                },
            );
            (saves, skipped)
        };
        let (saves, skipped) = run(None);
        // Every 5,000 files, naming the file not yet gone through.
        assert_eq!(
            saves,
            vec![
                (5_000, root.join("05000.js")),
                (10_000, root.join("10000.js"))
            ]
        );
        assert_eq!(skipped, crate::progress::Skipped::default());

        let resume = crate::progress::Resume {
            from: root.join("05000.js"),
            reread: vec![root.join("00010.js"), root.join("07000.js")],
        };
        let (saves, skipped) = run(Some(&resume));
        // Before the save point, only the file that had a finding is gone
        // through again; the one after it would be read anyway.
        assert_eq!(
            skipped,
            crate::progress::Skipped {
                skipped: 4_999,
                reread: 1
            }
        );
        // Positions are the walk's, so a continued pass saves where the
        // first one would have.
        assert_eq!(saves.len(), 2);
    }

    #[test]
    fn only_a_large_file_that_starts_like_a_program_is_hashed() {
        // ADR-0059: equal hashes mean equal bytes, so a file that does not
        // start like the programs on the list cannot match one of them.
        for program in [
            &b"\x7fELF\x02\x01"[..],
            b"\xcf\xfa\xed\xfe",
            b"\xfe\xed\xfa\xcf",
            b"\xca\xfe\xba\xbe",
            b"\xbe\xba\xfe\xca",
            b"MZ\x90\x00",
        ] {
            assert!(starts_like_program(program), "{program:?}");
        }
        for other in [
            &b"\x00\x00\x00\x18ftypmp42"[..],
            b"PK\x03\x04",
            b"%PDF",
            b"\x1f\x8b\x08\x00",
            b"",
            b"M",
        ] {
            assert!(!starts_like_program(other), "{other:?}");
        }

        let dir = scratch("sweep");
        let program = dir.join("Apps/renamed-tool");
        let video = dir.join("Media/holiday.mp4");
        let mut bytes = b"\x7fELF".to_vec();
        bytes.resize(11 * 1024 * 1024, 7);
        fs::create_dir_all(program.parent().expect("parent")).expect("dir");
        fs::write(&program, &bytes).expect("write");
        let mut movie = b"\x00\x00\x00\x18ftypmp42".to_vec();
        movie.resize(11 * 1024 * 1024, 7);
        fs::create_dir_all(video.parent().expect("parent")).expect("dir");
        fs::write(&video, &movie).expect("write");
        let ind = Indicators {
            strong: vec!["MARKER-ALPHA".into()],
            hashes: vec![(
                crate::sha256::file(&program).expect("hash"),
                "fixture".into(),
            )],
            ..Indicators::default()
        };
        let walk = Walk {
            files: vec![program.clone(), video],
            ..Walk::default()
        };
        let q = Quarantine::<DryRun>::new(dir.join("q"));
        let mut v = Verdict::new();
        let mut asked: Vec<(usize, usize)> = Vec::new();
        known_hashes(&walk, &ind, &mut v, &mut Sink::Dry(&q), &mut |n, of, _| {
            asked.push((n, of));
        });
        assert_eq!(
            asked,
            vec![(0, 1), (1, 1)],
            "one file hashed, the video passed over"
        );
        assert_eq!(v.hits(), 1);
        assert!(v.entries().iter().any(|e| matches!(e, crate::verdict::Entry::Note(n)
            if n.starts_with("Hashed 1 of the 2 files of 10 MB to 300 MB: the ones that start like a program."))));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_long_flat_config_is_not_a_payload_without_a_tell() {
        // Flat configs legitimately open with `export default [` and run long.
        // Length alone must not flag, or every large config is a finding.
        let ordinary = (0..200)
            .map(|i| format!("  rule{i},"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!looks_like_payload(&ordinary));
        assert!(looks_like_payload("const x = eval(atob('...'))"));
        assert!(looks_like_payload(&format!(
            "const p = '{}'",
            "A".repeat(600)
        )));
    }
}
