//! The report file, written as a scan goes. ADR-0059.
//!
//! Each line is appended and synced to the disk the moment it is written, so
//! a scan that is stopped, killed or loses its power has already saved what
//! it found. A machine whose battery dies takes the operating system's write
//! cache with it, which is why the file is synced and not only flushed.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub struct Report {
    path: PathBuf,
    file: Option<File>,
    /// The first error, said once at the end rather than on every line.
    failed: Option<io::Error>,
}

impl Report {
    /// Start the report at `path`, replacing any earlier one, with `opening`
    /// as its first lines. A report that cannot be created is remembered and
    /// said at the end; the scan still runs.
    pub fn create(path: &Path, opening: &str) -> Self {
        let mut report = Self {
            path: path.to_path_buf(),
            file: None,
            failed: None,
        };
        let opened = (|| {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent)?;
                }
            }
            OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)
        })();
        match opened {
            Ok(file) => report.file = Some(file),
            Err(e) => report.failed = Some(e),
        }
        report.write(opening);
        report
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append lines and sync them to the disk.
    pub fn lines(&mut self, lines: &[String]) {
        if lines.is_empty() {
            return;
        }
        let mut text = lines.join("\n");
        text.push('\n');
        self.write(&text);
    }

    /// Append text as it is, and sync it to the disk.
    pub fn write(&mut self, text: &str) {
        let Some(file) = self.file.as_mut() else {
            return;
        };
        if let Err(e) = file
            .write_all(text.as_bytes())
            .and_then(|()| file.sync_data())
        {
            self.failed.get_or_insert(e);
            self.file = None;
        }
    }

    /// The first error, if the report could not be written in full.
    pub fn error(&self) -> Option<&io::Error> {
        self.failed.as_ref()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn every_line_is_on_disk_the_moment_it_is_written() {
        let dir = std::env::temp_dir().join(format!("polinrider-report-{}", std::process::id()));
        let path = dir.join("sub/report.txt");
        let mut r = Report::create(&path, "opening\n");
        r.lines(&["one".to_string(), "two".to_string()]);
        // Read back while the report is still open: nothing waits for a close.
        assert_eq!(fs::read_to_string(&path).unwrap(), "opening\none\ntwo\n");
        assert!(r.error().is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_report_that_cannot_be_written_is_said_and_the_scan_goes_on() {
        let blocker =
            std::env::temp_dir().join(format!("polinrider-blocker-{}", std::process::id()));
        fs::write(&blocker, "a file, not a folder").unwrap();
        let mut r = Report::create(&blocker.join("report.txt"), "x\n");
        r.lines(&["y".to_string()]);
        assert!(r.error().is_some());
        let _ = fs::remove_file(&blocker);
    }
}
