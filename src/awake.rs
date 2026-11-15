//! Keeping the machine awake while a scan runs. ADR-0059.
//!
//! A check of a backup drive ran for a day and was lost when the laptop went
//! to sleep and then ran out of battery. While a scan runs, the system is
//! asked not to sleep for being idle, by a helper the system already has,
//! started with a fixed path and fixed arguments, nothing from a scanned
//! file. It ends when the scan ends, and on its own if this process dies.
//!
//! - macOS: `/usr/bin/caffeinate -i -w <pid>`, which ends when the pid does.
//! - Linux: `systemd-inhibit` holding an idle and sleep lock while the pid
//!   lives, where systemd is there to ask. Elsewhere nothing is done.
//! - Windows: nothing yet. Asking needs a system call this crate does not
//!   make (it forbids unsafe code), and is recorded in the ADR.
//!
//! Nothing here can stop a scan: a helper that is missing or fails to start
//! is passed over in silence.

use std::path::Path;
use std::process::{Child, Command, Stdio};

/// Held while a scan runs. Dropping it lets the system sleep again.
pub struct Awake(Option<Child>);

impl Awake {
    pub fn hold() -> Self {
        let pid = std::process::id().to_string();
        let child = if cfg!(target_os = "macos") {
            spawn("/usr/bin/caffeinate", &["-i", "-w", &pid])
        } else if cfg!(target_os = "linux") {
            let watch = format!("while kill -0 {pid} 2>/dev/null; do sleep 15; done");
            spawn(
                "/usr/bin/systemd-inhibit",
                &[
                    "--what=idle:sleep",
                    "--who=polinrider",
                    "--why=a scan is running",
                    "--mode=block",
                    "/bin/sh",
                    "-c",
                    &watch,
                ],
            )
        } else {
            None
        };
        Self(child)
    }
}

fn spawn(program: &str, args: &[&str]) -> Option<Child> {
    if !Path::new(program).is_file() {
        return None;
    }
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

impl Drop for Awake {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
