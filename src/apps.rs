//! The startup files of editors and desktop apps that the second stage
//! patches so that the payload runs again each time one of them opens.
//!
//! The November 2026 second stage writes a copy of the loader into VS Code,
//! Cursor, Antigravity, Discord and GitHub Desktop, leaves a `.inz.cjs` copy
//! beside each file it patches, and marks what it writes with markers that
//! are in `strong.txt` (Greenflagged, 2026-11-03, updated 2026-11-04).
//! Removing the package or cleaning the repository undoes none of it, and the
//! walk never enters `/Applications` or `AppData`. ADR-0064.
//!
//! The files are the ones Greenflagged's summary names, from the implant's
//! own string table: `out/main.js` for VS Code and Cursor,
//! `node_modules/@vscode/deviceid/dist/index.js` for VS Code, Cursor and
//! Antigravity, Discord's `modules/discord_desktop_core/index.js` and GitHub
//! Desktop's `main.js`. They are read at each app's install places on each
//! platform, under the home directory or the host's system root, so the
//! corpus can supply both as files (ADR-0029), the same shape as npm's own
//! program files (ADR-0042).
//!
//! What confirms a file is a marker or a known indicator in it, or a
//! `filenames.txt` name beside it. The two review rules, code after a long run
//! of blank space and a Discord `index.js` over 1 KB, are this tool's own
//! inference: no source says how the editor files are patched.

use crate::checks::Sink;
use crate::host::{Host, Platform, Probe};
use crate::indicators::Indicators;
use crate::runtimes::{expand, push_unique};
use crate::strip::MIN_PADDING;
use crate::verdict::{Finding, Kind, Verdict};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Which folder a place is under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Base {
    Home,
    System,
}

/// One install place of one app: a folder, a `/`-separated pattern in which
/// a component may hold one `*`, and the startup files in it.
struct Place {
    app: &'static str,
    base: Base,
    dir: &'static str,
    files: &'static [&'static str],
}

const MAIN_AND_DEVICEID: &[&str] = &["out/main.js", "node_modules/@vscode/deviceid/dist/index.js"];
const DEVICEID: &[&str] = &["node_modules/@vscode/deviceid/dist/index.js"];
const INDEX: &[&str] = &["index.js"];
const MAIN: &[&str] = &["main.js"];

const fn place(
    app: &'static str,
    base: Base,
    dir: &'static str,
    files: &'static [&'static str],
) -> Place {
    Place {
        app,
        base,
        dir,
        files,
    }
}

const VS_CODE: &str = "VS Code";
const CURSOR: &str = "Cursor";
const ANTIGRAVITY: &str = "Antigravity";
const DISCORD: &str = "Discord";
const GITHUB_DESKTOP: &str = "GitHub Desktop";

const MACOS: &[Place] = &[
    place(
        VS_CODE,
        Base::System,
        "Applications/Visual Studio Code.app/Contents/Resources/app",
        MAIN_AND_DEVICEID,
    ),
    place(
        VS_CODE,
        Base::Home,
        "Applications/Visual Studio Code.app/Contents/Resources/app",
        MAIN_AND_DEVICEID,
    ),
    place(
        CURSOR,
        Base::System,
        "Applications/Cursor.app/Contents/Resources/app",
        MAIN_AND_DEVICEID,
    ),
    place(
        CURSOR,
        Base::Home,
        "Applications/Cursor.app/Contents/Resources/app",
        MAIN_AND_DEVICEID,
    ),
    place(
        ANTIGRAVITY,
        Base::System,
        "Applications/Antigravity.app/Contents/Resources/app",
        DEVICEID,
    ),
    place(
        ANTIGRAVITY,
        Base::Home,
        "Applications/Antigravity.app/Contents/Resources/app",
        DEVICEID,
    ),
    place(
        GITHUB_DESKTOP,
        Base::System,
        "Applications/GitHub Desktop.app/Contents/Resources/app",
        MAIN,
    ),
    place(
        GITHUB_DESKTOP,
        Base::Home,
        "Applications/GitHub Desktop.app/Contents/Resources/app",
        MAIN,
    ),
    place(
        DISCORD,
        Base::Home,
        "Library/Application Support/discord/*/modules/discord_desktop_core",
        INDEX,
    ),
];

const LINUX: &[Place] = &[
    place(
        VS_CODE,
        Base::System,
        "usr/share/code/resources/app",
        MAIN_AND_DEVICEID,
    ),
    place(
        CURSOR,
        Base::System,
        "usr/share/cursor/resources/app",
        MAIN_AND_DEVICEID,
    ),
    place(
        ANTIGRAVITY,
        Base::System,
        "usr/share/antigravity/resources/app",
        DEVICEID,
    ),
    place(
        GITHUB_DESKTOP,
        Base::System,
        "usr/share/GitHub Desktop/resources/app",
        MAIN,
    ),
    place(
        GITHUB_DESKTOP,
        Base::System,
        "usr/lib/github-desktop/resources/app",
        MAIN,
    ),
    place(
        DISCORD,
        Base::Home,
        ".config/discord/*/modules/discord_desktop_core",
        INDEX,
    ),
    // Discord's packed app: no startup file to read by name, so only a side
    // file left in the folder is looked for.
    place(DISCORD, Base::System, "usr/share/discord", &[]),
];

const WINDOWS: &[Place] = &[
    place(
        VS_CODE,
        Base::Home,
        "AppData/Local/Programs/Microsoft VS Code/resources/app",
        MAIN_AND_DEVICEID,
    ),
    place(
        VS_CODE,
        Base::Home,
        "AppData/Local/Programs/Microsoft VS Code/*/resources/app",
        MAIN_AND_DEVICEID,
    ),
    place(
        CURSOR,
        Base::Home,
        "AppData/Local/Programs/cursor/resources/app",
        MAIN_AND_DEVICEID,
    ),
    place(
        ANTIGRAVITY,
        Base::Home,
        "AppData/Local/Programs/Antigravity/resources/app",
        DEVICEID,
    ),
    place(
        DISCORD,
        Base::Home,
        "AppData/Local/Discord/app-*/modules/discord_desktop_core-*/discord_desktop_core",
        INDEX,
    ),
    place(
        GITHUB_DESKTOP,
        Base::Home,
        "AppData/Local/GitHubDesktop/app-*/resources/app",
        MAIN,
    ),
];

fn places(platform: Platform) -> &'static [Place] {
    match platform {
        Platform::MacOs => MACOS,
        Platform::Linux => LINUX,
        Platform::Windows => WINDOWS,
    }
}

/// Discord's `index.js` as it ships is one line, `module.exports =
/// require('./core.asar');`. Larger than this is not that line.
const DISCORD_INDEX_MAX: u64 = 1024;

/// Why a startup file does not look the way its app ships it, if it does
/// not. Inference, not a source's description: ADR-0064.
fn altered(app: &str, file: &str, bytes: &[u8]) -> Option<String> {
    let size = bytes.len() as u64;
    if app == DISCORD && file == "index.js" && size > DISCORD_INDEX_MAX {
        return Some(format!(
            "{} KB, where Discord ships one line",
            size.div_ceil(1024)
        ));
    }
    let text = String::from_utf8_lossy(bytes);
    let padded = text.lines().any(|line| {
        let mut run = 0usize;
        for c in line.chars() {
            if c == ' ' || c == '\t' {
                run += 1;
            } else {
                if run >= MIN_PADDING {
                    return true;
                }
                run = 0;
            }
        }
        false
    });
    padded.then(|| "code after a long run of blank space".into())
}

/// The remedy under a confirmed finding about `app`.
fn reinstall(app: &str) -> String {
    format!(
        "the payload starts again each time {app} opens. Quit {app} and install it again from its maker's site before you open it."
    )
}

/// The startup files of the editors and apps this machine has in the usual
/// places, and what the second stage leaves beside them.
pub fn startup_files(
    home: &Path,
    host: &dyn Host,
    ind: &Indicators,
    v: &mut Verdict,
    sink: &mut Sink,
) {
    v.section("Editor and app startup files");
    let root = match host.system_root() {
        Probe::Read(root) => Some(root),
        Probe::NoTool(why) | Probe::Failed(why) => {
            v.push(Finding::review(format!(
                "system directories could not be read, so editors and apps installed for every user were not checked: {why}"
            )));
            None
        }
    };

    // Each install found, once, under any name it is reached by.
    let mut installs: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut found: Vec<(&Place, PathBuf)> = Vec::new();
    for p in places(host.platform()) {
        let base = match p.base {
            Base::Home => home,
            Base::System => match &root {
                Some(root) => root.as_path(),
                None => continue,
            },
        };
        for dir in expand(base, p.dir) {
            if !dir.is_dir() {
                continue;
            }
            let before = installs.len();
            push_unique(&mut installs, dir.clone());
            if installs.len() > before {
                found.push((p, dir));
            }
        }
    }

    let mut flagged = 0usize;
    for (p, dir) in &found {
        // The folders to look in for a side file: the install's own, and the
        // folder of each startup file.
        let mut beside: Vec<PathBuf> = vec![dir.clone()];
        for file in p.files {
            let path = file.split('/').fold(dir.clone(), |d, part| d.join(part));
            if let Some(parent) = path.parent() {
                if !beside.iter().any(|b| b == parent) {
                    beside.push(parent.to_path_buf());
                }
            }
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => {
                    flagged += 1;
                    v.push(Finding::review(format!(
                        "could not read, so it was not checked: {} ({e})",
                        path.display()
                    )));
                    continue;
                }
            };
            if ind.bytes_have_strong(&bytes) {
                flagged += 1;
                let line = sink.take(&path, "app-startup-file");
                v.push(
                    Finding::hit(
                        Kind::AppStartup,
                        format!(
                            "{}'s startup file contains an indicator: {}",
                            p.app,
                            path.display()
                        ),
                    )
                    .at(&path)
                    .with_remedy(reinstall(p.app))
                    .with_remedy(line),
                );
            } else if let Some(why) = altered(p.app, file, &bytes) {
                flagged += 1;
                v.push(
                    Finding::review(format!(
                        "{}'s startup file looks altered ({why}), verify it by hand: {}",
                        p.app,
                        path.display()
                    ))
                    .at(&path),
                );
            }
        }
        flagged += side_files(p.app, &beside, ind, v, sink);
    }

    if flagged == 0 {
        v.push(Finding::ok(match found.len() {
            0 => "no editor or app installation found in the usual places".to_string(),
            1 => "1 editor or app checked, its startup files are as shipped".to_string(),
            n => format!("{n} editors and apps checked, their startup files are as shipped"),
        }));
    }
}

/// The `.inz.*` files the second stage leaves beside a file it patches, in
/// each folder given. A `filenames.txt` name is confirmed, a
/// `filenames-review.txt` name is review. The walk never reaches these
/// folders, so this is the only place those names are looked for here.
fn side_files(
    app: &str,
    folders: &[PathBuf],
    ind: &Indicators,
    v: &mut Verdict,
    sink: &mut Sink,
) -> usize {
    let mut flagged = 0usize;
    for folder in folders {
        let Ok(entries) = fs::read_dir(folder) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.contains(".inz."))
            })
            .collect();
        paths.sort();
        for path in paths {
            let shown = path.to_string_lossy().replace('\\', "/");
            if ind.is_bad_filename(&shown) {
                flagged += 1;
                let line = sink.take(&path, "app-startup-file");
                v.push(
                    Finding::hit(
                        Kind::AppStartup,
                        format!(
                            "a copy of the loader sits beside {app}'s startup file: {}",
                            path.display()
                        ),
                    )
                    .at(&path)
                    .with_remedy(reinstall(app))
                    .with_remedy(line),
                );
            } else if ind.is_review_filename(&shown) {
                flagged += 1;
                v.push(
                    Finding::review(format!(
                        "a backup the payload leaves sits beside {app}'s startup file, verify it by hand: {}",
                        path.display()
                    ))
                    .at(&path),
                );
            }
        }
    }
    flagged
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn startup_files_as_shipped_are_not_altered() {
        let discord = b"module.exports = require('./core.asar');\n";
        assert_eq!(altered(DISCORD, "index.js", discord), None);
        let minified = format!("\"use strict\";{}", "var a=1;".repeat(20_000));
        assert_eq!(altered(VS_CODE, "out/main.js", minified.as_bytes()), None);
        // Trailing blanks alone are not code after them.
        let trailing = format!("var a = 1;{}\n", " ".repeat(300));
        assert_eq!(
            altered(GITHUB_DESKTOP, "main.js", trailing.as_bytes()),
            None
        );
    }

    #[test]
    fn padding_or_a_large_discord_index_is_altered() {
        let padded = format!("var a = 1;{}require('x')\n", "\t".repeat(200));
        assert_eq!(
            altered(GITHUB_DESKTOP, "main.js", padded.as_bytes()).as_deref(),
            Some("code after a long run of blank space")
        );
        let big = format!(
            "module.exports = require('./core.asar');\n{}",
            "x".repeat(2500)
        );
        assert_eq!(
            altered(DISCORD, "index.js", big.as_bytes()).as_deref(),
            Some("3 KB, where Discord ships one line")
        );
        // The size rule is Discord's alone: other files ship large.
        assert_eq!(altered(VS_CODE, "out/main.js", big.as_bytes()), None);
    }

    #[test]
    fn every_platform_names_each_app_and_greenflagged_s_files() {
        for platform in [Platform::MacOs, Platform::Linux, Platform::Windows] {
            let list = places(platform);
            for app in [VS_CODE, CURSOR, ANTIGRAVITY, DISCORD, GITHUB_DESKTOP] {
                assert!(list.iter().any(|p| p.app == app), "{app} on {platform:?}");
            }
            for p in list {
                let want: &[&str] = match p.app {
                    VS_CODE | CURSOR => MAIN_AND_DEVICEID,
                    ANTIGRAVITY => DEVICEID,
                    GITHUB_DESKTOP => MAIN,
                    _ if p.files.is_empty() => &[],
                    _ => INDEX,
                };
                assert_eq!(p.files, want, "{} at {}", p.app, p.dir);
            }
        }
    }
}
