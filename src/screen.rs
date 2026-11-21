//! What a scan shows while it runs: three steps, each finding the moment it
//! is made, and how far it has come. ADR-0059.
//!
//! Pure: it is handed the scan's events and returns lines. The binary and the
//! guided flow print them, and the report file is appended with the same
//! lines as they come, so a scan that is killed has already saved
//! everything it found.
//!
//! A live terminal gets a region drawn over in place: the step that is
//! running, its latest findings, the counts and the steps still waiting.
//! When a step ends, it is printed for good, header first and then every
//! finding it made. A pipe, a log and the report file get no region: each
//! step's header as it starts, each finding as it is made, and the step's
//! time as it ends. Standard output into a pipe or a log also gets a line
//! about once a minute while a step runs, saying how far it has come; the
//! report file does not (ADR-0070).

use crate::scan::{Event, STEPS};
use crate::verdict::{clean, Finding, Kind, Level};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The findings of the running step shown in the region; the rest are
/// counted, and all of them are printed when the step ends. A region taller
/// than the terminal cannot be drawn over.
const SHOWN: usize = 5;

/// The width the step names are padded to, so the states line up.
const NAME: usize = 39;

/// How long a step runs before a pipe is told it is still running, and
/// between one such line and the next. ADR-0070.
const BEAT: Duration = Duration::from_secs(60);

/// What to do with the screen after one event.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Frame {
    /// Lines to print for good, above the region.
    pub lines: Vec<String>,
    /// The region as it should now be drawn, on a live terminal. Empty when
    /// there is none.
    pub region: Vec<String>,
}

pub struct Screen {
    roots: Vec<PathBuf>,
    live: bool,
    step: usize,
    /// The running step's findings, as lines, in order.
    held: Vec<Vec<String>>,
    count: Option<String>,
    confirmed: usize,
    review: usize,
    /// This check continues one that stopped: step 2's count includes the
    /// files the earlier one read. ADR-0060.
    resumed: bool,
    /// Standard output into a pipe or a log: a line a minute while a step
    /// runs. ADR-0070.
    beating: bool,
    /// When, into the running step, the next such line is due.
    next_beat: Duration,
}

impl Screen {
    pub fn new(roots: &[PathBuf], live: bool) -> Self {
        Self {
            roots: roots.to_vec(),
            live,
            step: 0,
            held: Vec::new(),
            count: None,
            confirmed: 0,
            review: 0,
            resumed: false,
            beating: false,
            next_beat: BEAT,
        }
    }

    /// The screen of standard output: into a pipe or a log, it says once a
    /// minute that a long step is still running. A live terminal has its
    /// count redrawn instead, and the report file is not told. ADR-0070.
    pub fn beating(mut self) -> Self {
        self.beating = !self.live;
        self
    }

    /// The screen of a check that continues one that stopped.
    pub fn resumed(mut self) -> Self {
        self.resumed = true;
        self
    }

    pub fn event(&mut self, event: &Event) -> Frame {
        let mut frame = Frame::default();
        match *event {
            Event::Started(n) => {
                self.step = n;
                self.held.clear();
                self.count = None;
                self.next_beat = BEAT;
                if !self.live {
                    frame.lines.push(String::new());
                    frame.lines.push(header(n, ""));
                }
            }
            Event::Found(f) => {
                match f.level {
                    Level::Hit => self.confirmed += 1,
                    Level::Review => self.review += 1,
                    Level::Info | Level::Ok => return self.frame(frame),
                }
                let lines = self.finding(f);
                if !self.live {
                    frame.lines.extend(lines.iter().cloned());
                }
                self.held.push(lines);
            }
            Event::Count {
                step,
                n,
                of,
                elapsed,
            } => {
                if n > 0 || of.is_some() {
                    self.count = Some(match (step, of) {
                        (3, Some(of)) => {
                            format!("    {} of {} files hashed", thousands(n), thousands(of))
                        }
                        (1, _) => format!("    {} files listed so far", thousands(n)),
                        _ if self.resumed => format!(
                            "    {} files read so far, counting the earlier check",
                            thousands(n)
                        ),
                        _ => format!("    {} files read so far", thousands(n)),
                    });
                }
                self.beat(elapsed, &mut frame);
            }
            Event::Repos { n, of, elapsed } => {
                self.count = Some(format!(
                    "    {} of {} git repositories read",
                    thousands(n),
                    thousands(of)
                ));
                self.beat(elapsed, &mut frame);
            }
            Event::Done(n, took) => {
                if self.live {
                    frame.lines.push(String::new());
                    frame
                        .lines
                        .push(header(n, &format!("done, {}", duration(took))));
                    frame.lines.extend(self.held.iter().flatten().cloned());
                } else {
                    frame.lines.push(format!("    done, {}", duration(took)));
                }
                self.held.clear();
                self.count = None;
                self.step = n + 1;
            }
            Event::Save { .. } => {}
        }
        self.frame(frame)
    }

    /// Into a pipe, once a minute: the step is still running, and how far
    /// it has come. ADR-0070.
    fn beat(&mut self, elapsed: Duration, frame: &mut Frame) {
        if self.beating && elapsed >= self.next_beat {
            if let Some(count) = &self.count {
                frame.lines.push(format!(
                    "    still running after {}: {}",
                    duration(elapsed),
                    count.trim_start()
                ));
                self.next_beat = elapsed + BEAT;
            }
        }
    }

    fn frame(&self, mut frame: Frame) -> Frame {
        if !self.live || self.step == 0 || self.step > 3 {
            return frame;
        }
        let mut region = vec![String::new(), header(self.step, "running")];
        if let Some(count) = &self.count {
            region.push(count.clone());
        }
        let hidden = self.held.len().saturating_sub(SHOWN);
        if hidden > 0 {
            region.push(format!(
                "    {hidden} more found in this step, in the report"
            ));
        }
        region.extend(self.held.iter().skip(hidden).flatten().cloned());
        region.push(String::new());
        region.push(format!(
            "    {} confirmed, {} to review so far",
            self.confirmed, self.review
        ));
        for later in self.step + 1..=3 {
            region.push(String::new());
            region.push(header(later, "waiting"));
        }
        frame.region = region;
        frame
    }

    fn finding(&self, f: &Finding) -> Vec<String> {
        let tag = if f.level == Level::Hit {
            "CONFIRMED"
        } else {
            "TO REVIEW"
        };
        let what = match f.kind {
            Some(kind) => plain(kind, &f.message),
            None => clean(f.what()),
        };
        let mut lines = vec![format!("    {tag}  {what}")];
        if let Some(path) = &f.path {
            lines.push(format!("               {}", clean(&self.shown(path))));
        }
        lines
    }

    /// A path under a scanned root, from that root; anything else whole.
    fn shown(&self, path: &Path) -> String {
        self.roots
            .iter()
            .filter_map(|r| path.strip_prefix(r).ok())
            .filter(|p| !p.as_os_str().is_empty())
            .min_by_key(|p| p.as_os_str().len())
            .map_or_else(|| path.display().to_string(), |p| p.display().to_string())
    }
}

fn header(n: usize, state: &str) -> String {
    let name = STEPS.get(n.wrapping_sub(1)).copied().unwrap_or("");
    if state.is_empty() {
        format!("  Step {n} of 3  {name}")
    } else {
        format!("  Step {n} of 3  {name:<NAME$}{state}")
    }
}

/// A confirmed finding in the words of the screen.
fn plain(kind: Kind, message: &str) -> String {
    match kind {
        Kind::Config { .. } if message.starts_with("source file") => {
            "a source file carries the payload"
        }
        Kind::Config { .. } => "a build config carries the payload",
        Kind::TasksJson => "an editor task runs the payload when the folder opens",
        Kind::FakeFont => "a font file is really a script",
        Kind::Package => "a project depends on a campaign package",
        Kind::HijackedRelease => "a project installs a hijacked release of a real package",
        Kind::GitHook => "a git hook carries the payload",
        Kind::Implant => "the second-stage implant is on this disk",
        Kind::Propagation => "the script the payload spreads with is here",
        Kind::NamedFile => "a file carries a name the campaign uses",
        Kind::GitHistory => "a git repository still holds the payload",
        Kind::Extension => "an editor extension carries the payload",
        Kind::LoginItem => "something that starts at login runs the payload",
        Kind::Crontab => "a scheduled job runs the payload",
        Kind::StartupFile => "a shell startup file runs the payload",
        Kind::Registry => "an npm setting runs the payload",
        Kind::NpmProgram => "npm's own program files were changed to run the payload",
        Kind::PthFile => "a Python startup file runs the payload",
        Kind::AppStartup => "an editor or app was changed to run the payload",
        Kind::Autostart => "an entry that starts with Windows runs the payload",
        Kind::SshKey => "the attacker's SSH key can log in",
        Kind::Process => "the implant is running now",
        Kind::Connection => "a connection to the campaign is open now",
    }
    .to_string()
}

/// 1,284,551.
pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// 6s, 4m 10s, 2h 5m.
pub fn duration(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m {}s", s / 60, s % 60),
        _ => format!("{}h {}m", s / 3600, s % 3600 / 60),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn tasks(root: &Path) -> Finding {
        let path = root.join("Users/mandeep/Sites/shop/.vscode/tasks.json");
        Finding::hit(
            Kind::TasksJson,
            format!(
                "tasks.json runs on folder open AND contains an indicator: {}",
                path.display()
            ),
        )
        .at(path)
    }

    #[test]
    fn a_pipe_gets_each_step_and_each_finding_as_it_comes() {
        let root = PathBuf::from("/Volumes/OldMac");
        let mut s = Screen::new(std::slice::from_ref(&root), false);
        let mut out = Vec::new();
        for e in [
            Event::Started(1),
            Event::Found(&tasks(&root)),
            Event::Done(1, Duration::from_secs(6)),
        ] {
            out.extend(s.event(&e).lines);
        }
        assert_eq!(
            out,
            vec![
                "",
                "  Step 1 of 3  Things that run by themselves",
                "    CONFIRMED  an editor task runs the payload when the folder opens",
                "               Users/mandeep/Sites/shop/.vscode/tasks.json",
                "    done, 6s",
            ]
        );
    }

    #[test]
    fn a_live_terminal_draws_the_running_step_and_prints_it_whole_when_done() {
        let root = PathBuf::from("/Volumes/OldMac");
        let mut s = Screen::new(std::slice::from_ref(&root), true);
        let _ = s.event(&Event::Started(1));
        let f = s.event(&Event::Found(&tasks(&root)));
        assert!(f.lines.is_empty());
        let done = s.event(&Event::Done(1, Duration::from_secs(6)));
        assert_eq!(
            done.lines,
            vec![
                "",
                "  Step 1 of 3  Things that run by themselves          done, 6s",
                "    CONFIRMED  an editor task runs the payload when the folder opens",
                "               Users/mandeep/Sites/shop/.vscode/tasks.json",
            ]
        );
        let _ = s.event(&Event::Started(2));
        let f = s.event(&Event::Count {
            step: 2,
            n: 1_284_551,
            of: None,
            elapsed: Duration::from_secs(600),
        });
        assert_eq!(
            f.region,
            vec![
                "",
                "  Step 2 of 3  Project files                          running",
                "    1,284,551 files read so far",
                "",
                "    1 confirmed, 0 to review so far",
                "",
                "  Step 3 of 3  Large files against known hashes       waiting",
            ]
        );
    }

    #[test]
    fn a_pipe_is_told_once_a_minute_that_a_long_step_is_still_running() {
        let root = PathBuf::from("/Volumes/OldMac");
        let count = |step, n, of, secs| Event::Count {
            step,
            n,
            of,
            elapsed: Duration::from_secs(secs),
        };
        let mut s = Screen::new(std::slice::from_ref(&root), false).beating();
        let mut out = Vec::new();
        let _ = s.event(&Event::Started(2));
        for (n, secs) in [
            (1024, 30),
            (61_440, 59),
            (62_464, 61),
            (90_112, 90),
            (118_784, 125),
        ] {
            out.extend(s.event(&count(2, n, None, secs)).lines);
        }
        assert_eq!(
            out,
            vec![
                "    still running after 1m 1s: 62,464 files read so far",
                "    still running after 2m 5s: 118,784 files read so far",
            ]
        );
        // The git part of step 2 counts repositories.
        assert_eq!(
            s.event(&Event::Repos {
                n: 12,
                of: 410,
                elapsed: Duration::from_secs(242),
            })
            .lines,
            vec!["    still running after 4m 2s: 12 of 410 git repositories read"]
        );
        // Each step starts its own minute, in its own words.
        let _ = s.event(&Event::Done(2, Duration::from_secs(130)));
        let _ = s.event(&Event::Started(3));
        assert_eq!(
            s.event(&count(3, 14, Some(61), 120)).lines,
            vec!["    still running after 2m 0s: 14 of 61 files hashed"]
        );
        let mut s = Screen::new(std::slice::from_ref(&root), false).beating();
        let _ = s.event(&Event::Started(1));
        assert_eq!(
            s.event(&count(1, 98_304, None, 60)).lines,
            vec!["    still running after 1m 0s: 98,304 files listed so far"]
        );
        let mut s = Screen::new(std::slice::from_ref(&root), false)
            .resumed()
            .beating();
        let _ = s.event(&Event::Started(2));
        assert_eq!(
            s.event(&count(2, 61_440, None, 60)).lines,
            vec!["    still running after 1m 0s: 61,440 files read so far, counting the earlier check"]
        );
        // The report file and a live terminal never get the line.
        let mut file = Screen::new(std::slice::from_ref(&root), false);
        let mut live = Screen::new(std::slice::from_ref(&root), true).beating();
        for s in [&mut file, &mut live] {
            let _ = s.event(&Event::Started(2));
            assert!(s.event(&count(2, 61_440, None, 600)).lines.is_empty());
        }
    }

    #[test]
    fn numbers_and_times_read_as_people_write_them() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1_284_551), "1,284,551");
        assert_eq!(duration(Duration::from_secs(6)), "6s");
        assert_eq!(duration(Duration::from_secs(250)), "4m 10s");
        assert_eq!(duration(Duration::from_secs(7500)), "2h 5m");
    }
}
