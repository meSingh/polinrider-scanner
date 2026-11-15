//! Where a stopped check had got to, so that the next one can continue it.
//! ADR-0060.
//!
//! A small file beside the report, `<report stem>.progress`, rewritten every
//! [`SAVE_EVERY`] files of step 2's pass over the project files. That pass
//! goes through the walk's files in sorted order, so how far it has come is
//! one path: everything that sorts before it is done. The file holds the last
//! two such save points, the paths of the files that pass found something in,
//! and what the run was (version, indicators, folders, options), so that only
//! a run that matches it can use it. No list of the machine's files.
//!
//! A run that continues one goes back to the older of the two save points,
//! reads again every file that had a finding, and runs everything else in
//! full. Its result is made only from its own reads.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// How many files of the pass between two save points.
pub const SAVE_EVERY: usize = 5_000;

const FORMAT: &str = "polinrider progress 1";

/// A position in the pass: the file at `at` is the `n`th, and every file
/// that sorts before it was gone through. Written at `when`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Point {
    pub n: usize,
    pub when: u64,
    pub at: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub version: String,
    /// SHA-256 of the indicator files the run loaded.
    pub indicators: String,
    /// `check` or `guide`. A guided check is continued only by another.
    pub mode: String,
    /// What else decides what a run looks at, one per line.
    pub options: Vec<String>,
    pub roots: Vec<PathBuf>,
    pub report: PathBuf,
    /// The process writing it. A check still running is never continued.
    pub pid: u32,
    pub started: u64,
    /// The step the run was in when this was written.
    pub step: usize,
    /// Files of the pass gone through when this was written.
    pub files: usize,
    pub saved: u64,
    /// At most two, the older first.
    pub points: Vec<Point>,
    /// Files the pass found something in.
    pub found: Vec<PathBuf>,
}

/// What a run is, to compare with what an earlier one was.
#[derive(Debug, Clone)]
pub struct Run {
    pub version: String,
    pub indicators: String,
    pub mode: String,
    pub options: Vec<String>,
    pub roots: Vec<PathBuf>,
    pub report: PathBuf,
}

impl Run {
    /// The progress file this run writes, before its first save point.
    pub fn progress(&self, now: u64) -> Progress {
        Progress {
            version: self.version.clone(),
            indicators: self.indicators.clone(),
            mode: self.mode.clone(),
            options: self.options.clone(),
            roots: self.roots.clone(),
            report: self.report.clone(),
            pid: std::process::id(),
            started: now,
            step: 2,
            files: 0,
            saved: now,
            points: Vec::new(),
            found: Vec::new(),
        }
    }
}

/// Seconds since 1970, now.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The progress file that belongs to a report.
pub fn path_for(report: &Path) -> PathBuf {
    report.with_extension("progress")
}

/// A digest of the indicator files in `dir`. A run with other indicators
/// must read again what an earlier one read: a new indicator has to see the
/// files already read.
pub fn indicators_digest(dir: &Path) -> String {
    let mut all = Vec::new();
    for name in crate::indicators::FILES {
        all.extend_from_slice(name.as_bytes());
        all.push(0);
        match fs::read(dir.join(name)) {
            Ok(bytes) => {
                all.extend_from_slice(bytes.len().to_string().as_bytes());
                all.push(0);
                all.extend_from_slice(&bytes);
            }
            Err(_) => all.extend_from_slice(b"absent"),
        }
        all.push(0);
    }
    crate::sha256::bytes(&all)
}

impl Progress {
    /// Note a save point, keeping the last two.
    pub fn save(&mut self, n: usize, at: &Path, found: &[PathBuf], now: u64) {
        self.points.push(Point {
            n,
            when: now,
            at: at.to_path_buf(),
        });
        if self.points.len() > 2 {
            self.points.remove(0);
        }
        self.files = n;
        self.saved = now;
        self.found = found.to_vec();
    }

    /// Where a run that continues this one starts its pass: the older of the
    /// two save points, a couple of steps behind where it stopped.
    pub fn from(&self) -> Option<&Point> {
        self.points.first()
    }

    pub fn to_text(&self) -> String {
        let mut out = format!("{FORMAT}\n");
        let mut line = |key: &str, value: &str| {
            out.push_str(key);
            out.push(' ');
            out.push_str(value);
            out.push('\n');
        };
        line("version", &escape(self.version.as_bytes()));
        line("indicators", &self.indicators);
        line("mode", &self.mode);
        for o in &self.options {
            line("option", &escape(o.as_bytes()));
        }
        for r in &self.roots {
            line("root", &escape_path(r));
        }
        line("report", &escape_path(&self.report));
        line("pid", &self.pid.to_string());
        line("started", &self.started.to_string());
        line("step", &self.step.to_string());
        line("files", &self.files.to_string());
        line("saved", &self.saved.to_string());
        for p in &self.points {
            line(
                "point",
                &format!("{} {} {}", p.n, p.when, escape_path(&p.at)),
            );
        }
        for f in &self.found {
            line("found", &escape_path(f));
        }
        out.push_str("end\n");
        out
    }

    /// Read a progress file's text. `None` for anything that is not whole and
    /// well formed: a damaged file is never half used.
    pub fn parse(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        if lines.next()? != FORMAT {
            return None;
        }
        let (mut version, mut indicators, mut mode, mut report) = (None, None, None, None);
        let (mut pid, mut started, mut step, mut files, mut saved) = (None, None, None, None, None);
        let (mut options, mut roots, mut points, mut found) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut ended = false;
        for l in lines {
            if ended {
                return None;
            }
            if l == "end" {
                ended = true;
                continue;
            }
            let (key, value) = l.split_once(' ')?;
            match key {
                "version" => version = Some(unescape_str(value)?),
                "indicators" => indicators = Some(value.to_string()),
                "mode" => mode = Some(value.to_string()),
                "option" => options.push(unescape_str(value)?),
                "root" => roots.push(unescape_path(value)?),
                "report" => report = Some(unescape_path(value)?),
                "pid" => pid = Some(value.parse().ok()?),
                "started" => started = Some(value.parse().ok()?),
                "step" => step = Some(value.parse().ok()?),
                "files" => files = Some(value.parse().ok()?),
                "saved" => saved = Some(value.parse().ok()?),
                "point" => {
                    let mut parts = value.splitn(3, ' ');
                    let n = parts.next()?.parse().ok()?;
                    let when = parts.next()?.parse().ok()?;
                    let at = unescape_path(parts.next()?)?;
                    points.push(Point { n, when, at });
                }
                "found" => found.push(unescape_path(value)?),
                _ => return None,
            }
        }
        if !ended || roots.is_empty() || points.is_empty() || points.len() > 2 {
            return None;
        }
        Some(Self {
            version: version?,
            indicators: indicators?,
            mode: mode?,
            options,
            roots,
            report: report?,
            pid: pid?,
            started: started?,
            step: step?,
            files: files?,
            saved: saved?,
            points,
            found,
        })
    }

    /// Write it beside the report: in full to a `.part` file, synced, then
    /// moved over the old one, so the file is always one whole version.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(".part");
        let part = path.with_file_name(name);
        let done = (|| {
            let mut f = File::create(&part)?;
            f.write_all(self.to_text().as_bytes())?;
            f.sync_all()?;
            fs::rename(&part, path)
        })();
        if done.is_err() {
            let _ = fs::remove_file(&part);
        }
        done
    }
}

/// What was found beside this run's report, or in the folder it goes in.
#[derive(Debug)]
pub enum Earlier {
    /// Nothing to continue.
    None,
    /// An unfinished check of these folders that this run may continue.
    Usable {
        progress: Box<Progress>,
        path: PathBuf,
    },
    /// An unfinished check of these folders that cannot be continued.
    Refused { why: Refusal, path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Version(String),
    Indicators,
    Folders,
    Options,
    Damaged,
    ReportMissing,
}

impl Refusal {
    /// The second line of the screen that says so (ADR-0060).
    pub fn said(&self) -> String {
        match self {
            Refusal::Version(v) => format!(
                "  it was made by PolinRider {}, so the part it read must be read again.",
                v.split(' ').next().unwrap_or(v)
            ),
            Refusal::Indicators => {
                "  it used older indicators, so the part it read must be read again.".into()
            }
            Refusal::Folders => "  it checked different folders.".into(),
            Refusal::Options => "  it used different options.".into(),
            Refusal::Damaged => "  its progress file is damaged.".into(),
            Refusal::ReportMissing => "  its report is missing.".into(),
        }
    }
}

/// Look for an unfinished check this run could continue: the progress file
/// of this run's own report first (a report named with `--report`), then any
/// in the same folder whose folders are this run's. A check still running is
/// left alone, and so is one of another mode.
pub fn find(run: &Run) -> Earlier {
    let own = path_for(&run.report);
    if own.is_file() {
        return match fs::read_to_string(&own)
            .ok()
            .as_deref()
            .and_then(Progress::parse)
        {
            None => Earlier::Refused {
                why: Refusal::Damaged,
                path: own,
            },
            Some(p) if p.mode != run.mode || running(p.pid) => Earlier::None,
            Some(p) if p.roots != run.roots => Earlier::Refused {
                why: Refusal::Folders,
                path: own,
            },
            Some(p) => judge(run, p, own),
        };
    }
    let dir = run
        .report
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let Ok(entries) = fs::read_dir(dir) else {
        return Earlier::None;
    };
    let mut best: Option<(Progress, PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("progress") {
            continue;
        }
        let Some(p) = fs::read_to_string(&path)
            .ok()
            .as_deref()
            .and_then(Progress::parse)
        else {
            continue;
        };
        if p.mode != run.mode || p.roots != run.roots || running(p.pid) {
            continue;
        }
        if best.as_ref().is_none_or(|(b, _)| p.saved > b.saved) {
            best = Some((p, path));
        }
    }
    match best {
        Some((p, path)) => judge(run, p, path),
        None => Earlier::None,
    }
}

fn judge(run: &Run, p: Progress, path: PathBuf) -> Earlier {
    let why = if p.version != run.version {
        Refusal::Version(p.version.clone())
    } else if p.indicators != run.indicators {
        Refusal::Indicators
    } else if p.options != run.options {
        Refusal::Options
    } else if path_for(&p.report) != path {
        // A progress file belongs to one report, beside it.
        Refusal::Damaged
    } else {
        match report_state(&p.report) {
            Report::Missing => Refusal::ReportMissing,
            // The check it belongs to did finish: nothing to continue.
            Report::Finished => {
                let _ = fs::remove_file(&path);
                return Earlier::None;
            }
            Report::InProgress => {
                return Earlier::Usable {
                    progress: Box::new(p),
                    path,
                }
            }
        }
    };
    Earlier::Refused { why, path }
}

enum Report {
    Missing,
    Finished,
    InProgress,
}

/// A report's first line says it is in progress until its verdict is
/// written (ADR-0059).
fn report_state(report: &Path) -> Report {
    use std::io::BufRead;
    let Ok(f) = File::open(report) else {
        return Report::Missing;
    };
    let mut first = String::new();
    let _ = io::BufReader::new(f).read_line(&mut first);
    if first.contains(", in progress. ") {
        Report::InProgress
    } else {
        Report::Finished
    }
}

/// Whether another process with this id is alive. Our own id never counts:
/// the run that wrote it is gone if we have its number.
fn running(pid: u32) -> bool {
    if pid == std::process::id() || pid == 0 {
        return false;
    }
    if cfg!(unix) && Path::new("/bin/sh").is_file() {
        return std::process::Command::new("/bin/sh")
            .args(["-c", "kill -0 \"$0\" 2>/dev/null", &pid.to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
    }
    false
}

/// Keeps this run's progress file as the scan goes: written at each save
/// point and when step 3 starts, removed when the check finishes. The
/// earlier run's file, when this run continues one, is removed at this run's
/// first save point, which supersedes it; until then a stop leaves it to be
/// continued again.
pub struct Keeper {
    path: PathBuf,
    earlier: Option<PathBuf>,
    progress: Progress,
    written: bool,
    files: usize,
}

impl Keeper {
    pub fn new(run: &Run, earlier: Option<PathBuf>, now: u64) -> Self {
        Self {
            path: path_for(&run.report),
            earlier,
            progress: run.progress(now),
            written: false,
            files: 0,
        }
    }

    pub fn event(&mut self, event: &crate::scan::Event, now: u64) {
        use crate::scan::Event;
        match *event {
            Event::Count { step: 2, n, .. } => self.files = n,
            Event::Save { n, at, found } => {
                self.progress.save(n, at, found, now);
                // A file that cannot be written leaves nothing to continue
                // from, which is the safe side: the next check starts again.
                if self.progress.write(&self.path).is_ok() {
                    self.written = true;
                    if let Some(earlier) = self.earlier.take() {
                        if earlier != self.path {
                            let _ = fs::remove_file(earlier);
                        }
                    }
                }
            }
            Event::Started(3) if self.written => {
                self.progress.step = 3;
                self.progress.files = self.files;
                self.progress.saved = now;
                let _ = self.progress.write(&self.path);
            }
            _ => {}
        }
    }

    /// The check finished: nothing is left to continue.
    pub fn finish(self) {
        if self.written {
            let _ = fs::remove_file(&self.path);
        }
        if let Some(earlier) = self.earlier {
            let _ = fs::remove_file(earlier);
        }
    }
}

// --- paths in a line of text ------------------------------------------------

/// Bytes a line can carry as they are; the rest as `%XX`. Every byte that is
/// not printable ASCII is escaped, so a name in any encoding survives.
fn escape(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if (0x20..0x7f).contains(&b) && b != b'%' {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn unescape(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    Some(out)
}

fn unescape_str(s: &str) -> Option<String> {
    String::from_utf8(unescape(s)?).ok()
}

#[cfg(unix)]
fn escape_path(p: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    escape(p.as_os_str().as_bytes())
}

#[cfg(unix)]
fn unescape_path(s: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(unescape(s)?)))
}

// Windows paths are UTF-16. One that is not valid Unicode is written with a
// replacement character, never matches a real path and so is never skipped.
#[cfg(not(unix))]
fn escape_path(p: &Path) -> String {
    escape(p.to_string_lossy().as_bytes())
}

#[cfg(not(unix))]
fn unescape_path(s: &str) -> Option<PathBuf> {
    unescape_str(s).map(PathBuf::from)
}

// --- what is said -------------------------------------------------------------

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// A moment as people say it, in this machine's time zone: "14 Nov at
/// 18:02", or with the year. In UTC, and saying so, where the zone cannot be
/// read.
pub fn when(seconds: u64, offset: Option<i64>, year: bool) -> String {
    let local = seconds.saturating_add_signed(offset.unwrap_or(0));
    let (y, m, d) = civil(local / 86_400);
    let rest = local % 86_400;
    let month = MONTHS.get(m.saturating_sub(1) as usize).unwrap_or(&"");
    format!(
        "{d} {month}{} at {:02}:{:02}{}",
        if year { format!(" {y}") } else { String::new() },
        rest / 3_600,
        rest % 3_600 / 60,
        if offset.is_some() { "" } else { " UTC" }
    )
}

/// The day only: "14 Nov".
pub fn day(seconds: u64, offset: Option<i64>) -> String {
    let local = seconds.saturating_add_signed(offset.unwrap_or(0));
    let (_, m, d) = civil(local / 86_400);
    let month = MONTHS.get(m.saturating_sub(1) as usize).unwrap_or(&"");
    format!("{d} {month}")
}

/// Days since 1970 to a calendar date. The same arithmetic as the
/// quarantine's stamp.
fn civil(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + u64::from(m <= 2), m, d)
}

/// This machine's offset from UTC in seconds, asked of `date` by its fixed
/// path. None where there is no such program, as on Windows.
pub fn utc_offset() -> Option<i64> {
    let date = Path::new("/bin/date");
    if !date.is_file() {
        return None;
    }
    let out = std::process::Command::new(date)
        .arg("+%z")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let z = String::from_utf8(out.stdout).ok()?;
    let z = z.trim();
    let sign = match z.get(..1)? {
        "+" => 1,
        "-" => -1,
        _ => return None,
    };
    let h: i64 = z.get(1..3)?.parse().ok()?;
    let m: i64 = z.get(3..5)?.parse().ok()?;
    Some(sign * (h * 3_600 + m * 60))
}

/// Screen A: the question asked on a live terminal. The answer is a word.
pub fn question(p: &Progress, offset: Option<i64>) -> Vec<String> {
    vec![
        String::new(),
        "  An earlier check of these folders did not finish.".into(),
        format!(
            "  It started on {} and stopped in step {}, after {} files.",
            when(p.started, offset, false),
            p.step,
            crate::screen::thousands(p.files)
        ),
        String::new(),
        "  Continue from where it stopped?".into(),
        String::new(),
        "    yes   carry on from there, and redo the quick checks".into(),
        "    no    start again from the beginning".into(),
        String::new(),
    ]
}

/// Screen B: what continuing does.
pub fn continuing(p: &Progress, offset: Option<i64>) -> Vec<String> {
    let back = match (p.points.first(), p.points.last()) {
        (Some(from), Some(last)) => last.n - from.n + SAVE_EVERY,
        _ => SAVE_EVERY,
    };
    vec![
        String::new(),
        format!(
            "  Continuing the check that stopped on {}.",
            when(p.saved, offset, false)
        ),
        "  Steps 1 and 3 run again in full. They are quick.".into(),
        format!(
            "  Step 2 goes back about {} files before the stop and carries on from there.",
            crate::screen::thousands(back)
        ),
        "  Files that had a finding are read again.".into(),
    ]
}

/// Screen C: into a pipe or a log, never asked.
pub fn not_asked() -> Vec<String> {
    vec![
        String::new(),
        "  An earlier check of these folders did not finish. Starting again from the".into(),
        "  beginning. To continue it instead, run the same command with --resume.".into(),
    ]
}

/// Screen D: it cannot be continued.
pub fn refused(why: &Refusal) -> Vec<String> {
    vec![
        String::new(),
        "  An earlier check of these folders did not finish, but it cannot be continued:".into(),
        why.said(),
        String::new(),
        "  Starting again from the beginning.".into(),
    ]
}

/// Screen D, for `--resume` with nothing to continue.
pub fn nothing_to_resume() -> Vec<String> {
    vec![
        String::new(),
        "  There is no unfinished check of these folders to continue.".into(),
        "  Starting from the beginning.".into(),
    ]
}

/// Where a check that continues another picks up the pass over the project
/// files: the older save point, and the files that had a finding, sorted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resume {
    pub from: PathBuf,
    pub reread: Vec<PathBuf>,
}

impl Progress {
    pub fn resume(&self) -> Option<Resume> {
        let mut reread = self.found.clone();
        reread.sort();
        reread.dedup();
        Some(Resume {
            from: self.from()?.at.clone(),
            reread,
        })
    }
}

/// What a resumed check's pass did, for section E of its report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Skipped {
    /// Files before the save point that were not read again.
    pub skipped: usize,
    /// Files before it that were read again because they had a finding.
    pub reread: usize,
}

/// Section E, in the report of a check that continued another.
pub fn resumed_section(p: &Progress, skipped: Skipped, offset: Option<i64>) -> String {
    let (Some(from), Some(last)) = (p.points.first(), p.points.last()) else {
        return String::new();
    };
    let again = match skipped.reread {
        0 => String::new(),
        1 => ", and the 1 file that had a finding".to_string(),
        n => format!(", and the {n} files that had a finding"),
    };
    format!(
        "\n== Resumed ==\n  This check continues one that stopped on {}.\n  Steps 1 and 3 ran again in full.\n  Step 2 read again from {} ({} files before the stop) to the end{again}.\n  The {} files before that point were read on {} and had no finding. A file added there since then was not read.\n",
        when(p.saved, offset, true),
        from.at.display(),
        crate::screen::thousands(last.n - from.n),
        crate::screen::thousands(skipped.skipped),
        day(from.when, offset),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn sample() -> Progress {
        let mut p = Run {
            version: "3.1.0".into(),
            indicators: "ab".repeat(32),
            mode: "check".into(),
            options: vec!["fs-only".into(), "home /home/m".into()],
            roots: vec![PathBuf::from("/Volumes/Old Mac"), PathBuf::from("/srv/x")],
            report: PathBuf::from("/home/m/polinrider-report-1.txt"),
        }
        .progress(1_763_143_320);
        p.save(5_000, Path::new("/srv/x/a b/5.js"), &[], 1_763_143_400);
        p.save(
            10_000,
            Path::new("/srv/x/c%d/\u{e9}\n.js"),
            &[PathBuf::from("/srv/x/a/1.js")],
            1_763_143_500,
        );
        p
    }

    #[test]
    fn a_progress_file_reads_back_as_it_was_written() {
        let p = sample();
        let text = p.to_text();
        assert!(text.starts_with("polinrider progress 1\nversion 3.1.0\n"));
        // One record per line, whatever the names hold.
        assert!(text.contains("point 10000 1763143500 /srv/x/c%25d/%C3%A9%0A.js\n"));
        assert_eq!(Progress::parse(&text), Some(p));
    }

    #[test]
    fn only_the_last_two_save_points_are_kept() {
        let mut p = sample();
        p.save(15_000, Path::new("/srv/x/z.js"), &[], 1_763_143_600);
        assert_eq!(
            p.points.iter().map(|x| x.n).collect::<Vec<_>>(),
            vec![10_000, 15_000]
        );
        assert_eq!(p.from().map(|x| x.n), Some(10_000));
        assert_eq!(p.files, 15_000);
    }

    #[test]
    fn a_damaged_progress_file_is_not_used_at_all() {
        let text = sample().to_text();
        // Cut short, as a full disk would leave it.
        assert_eq!(Progress::parse(&text[..text.len() / 2]), None);
        // No end line.
        assert_eq!(Progress::parse(text.trim_end_matches("end\n")), None);
        // Anything after the end.
        assert_eq!(Progress::parse(&format!("{text}root /x\n")), None);
        // A line it does not know.
        assert_eq!(Progress::parse(&text.replace("pid ", "pids ")), None);
        // A number that is not one.
        assert_eq!(
            Progress::parse(&text.replace("files 10000", "files ten")),
            None
        );
        // A broken escape, and one cut short at the end of a line.
        assert_eq!(Progress::parse(&text.replace("%25", "%G5")), None);
        assert_eq!(Progress::parse(&text.replace("%0A.js", "%0")), None);
        // Another format, or none.
        assert_eq!(
            Progress::parse(&text.replace("progress 1", "progress 2")),
            None
        );
        assert_eq!(Progress::parse(""), None);
        // No save point: nothing to continue from.
        let mut none = sample();
        none.points.clear();
        assert_eq!(Progress::parse(&none.to_text()), None);
    }

    #[test]
    fn the_progress_file_sits_beside_its_report() {
        assert_eq!(
            path_for(Path::new("/h/polinrider-report-20261115T101200Z.txt")),
            PathBuf::from("/h/polinrider-report-20261115T101200Z.progress")
        );
    }

    #[test]
    fn written_whole_and_nothing_left_beside_it() {
        let dir = std::env::temp_dir().join(format!("polinrider-progress-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("r.progress");
        let p = sample();
        p.write(&path).unwrap();
        p.write(&path).unwrap();
        assert_eq!(
            Progress::parse(&fs::read_to_string(&path).unwrap()),
            Some(p)
        );
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn times_read_as_people_say_them() {
        // 2025-11-14 18:02 UTC.
        let t = 1_763_143_320;
        assert_eq!(when(t, Some(0), false), "14 Nov at 18:02");
        assert_eq!(when(t, Some(0), true), "14 Nov 2025 at 18:02");
        assert_eq!(when(t, Some(19_800), false), "14 Nov at 23:32");
        assert_eq!(when(t, None, false), "14 Nov at 18:02 UTC");
        assert_eq!(day(t, Some(0)), "14 Nov");
    }

    #[test]
    fn the_question_and_what_continuing_does_read_as_the_mock() {
        let mut p = sample();
        p.files = 1_204_331;
        assert_eq!(
            question(&p, Some(0))[1..5],
            [
                "  An earlier check of these folders did not finish.".to_string(),
                "  It started on 14 Nov at 18:02 and stopped in step 2, after 1,204,331 files."
                    .to_string(),
                String::new(),
                "  Continue from where it stopped?".to_string(),
            ]
        );
        assert_eq!(
            continuing(&p, Some(0))[1..],
            [
                "  Continuing the check that stopped on 14 Nov at 18:05.".to_string(),
                "  Steps 1 and 3 run again in full. They are quick.".to_string(),
                "  Step 2 goes back about 10,000 files before the stop and carries on from there."
                    .to_string(),
                "  Files that had a finding are read again.".to_string(),
            ]
        );
    }

    #[test]
    fn the_resumed_section_says_what_was_and_was_not_read_again() {
        let p = sample();
        let s = resumed_section(
            &p,
            Skipped {
                skipped: 4_999,
                reread: 1,
            },
            Some(0),
        );
        assert_eq!(
            s,
            "\n== Resumed ==\n  This check continues one that stopped on 14 Nov 2025 at 18:05.\n  Steps 1 and 3 ran again in full.\n  Step 2 read again from /srv/x/a b/5.js (5,000 files before the stop) to the end, and the 1 file that had a finding.\n  The 4,999 files before that point were read on 14 Nov and had no finding. A file added there since then was not read.\n"
        );
    }
}
