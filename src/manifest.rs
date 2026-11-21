//! The package names a manifest or a lockfile depends on, read as data.
//!
//! `ioc/bad-packages.txt` used to be matched as fixed strings against the whole
//! file, the way `grep -F` would. That reports a project depending on
//! `sass-formatter` as depending on a campaign package called `sass-format`,
//! and a description that mentions a campaign package as a dependency on it.
//! The weekly reviews turned eight known-malicious names away for that reason.
//! Here each file is read for the names it actually installs, and a name is
//! compared whole. See ADR-0041.
//!
//! Four formats, each read only as far as it has to be:
//!
//! - `package.json` and `package-lock.json` are JSON, parsed by a small parser
//!   below. It builds no recursion on the call stack while parsing and refuses
//!   anything nested deeper than [`MAX_DEPTH`], so a hostile file cannot
//!   exhaust the stack.
//! - `yarn.lock` (v1 and berry) and `pnpm-lock.yaml` are read line by line.
//!   Neither needs a YAML parser to give up its package names.
//!
//! A file that cannot be read as its format is an error, never an empty list.
//! The caller falls back to matching the text and says that it did.
//!
//! Pure: no I/O, no indicators. It answers "what does this install".

/// Deeper than any real manifest, and shallow enough that walking the parsed
/// value recursively afterwards is safe.
pub const MAX_DEPTH: usize = 256;

/// The four keys under which a manifest lists what it installs.
const DEPENDENCY_KEYS: &[&str] = &[
    "dependencies",
    "devDependencies",
    "optionalDependencies",
    "peerDependencies",
];

/// Is this a file whose package names are read here?
pub fn is_manifest(name: &str) -> bool {
    matches!(
        name,
        "package.json" | "package-lock.json" | "pnpm-lock.yaml" | "yarn.lock"
    )
}

/// Every package name `text` depends on or resolves, for a file called
/// `file_name`. Duplicates are left in; order is the file's. `Err` says why
/// the file could not be read as what its name says it is.
pub fn package_names(file_name: &str, text: &str) -> Result<Vec<String>, String> {
    match file_name {
        "package.json" | "package-lock.json" => {
            let root = parse_json(text)?;
            json_names(&root)
        }
        "yarn.lock" => yarn_names(text),
        "pnpm-lock.yaml" => pnpm_names(text),
        other => Err(format!("{other} is not a manifest")),
    }
}

/// Every release, `(name, version)`, that `text` resolves or pins, for a file
/// called `file_name`. A lockfile gives what it resolved; a `package.json`
/// gives its own name and version, and each dependency's value as written,
/// so an exact pin compares equal to a release and a range never does.
/// `Err` as for [`package_names`]. ADR-0063.
pub fn package_releases(file_name: &str, text: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = match file_name {
        "package.json" | "package-lock.json" => {
            let root = parse_json(text)?;
            json_releases(&root)?
        }
        "yarn.lock" => yarn_releases(text)?,
        "pnpm-lock.yaml" => pnpm_releases(text)?,
        other => return Err(format!("{other} is not a manifest")),
    };
    out.retain(|(n, v)| !n.is_empty() && !v.is_empty());
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

/// A version as a lockfile or a pin writes it: `=1.2.3` and `v1.2.3` are
/// `1.2.3`, and a peer suffix (`1.2.3(react@18)`, pnpm) is not part of it.
fn bare_version(v: &str) -> String {
    let v = v.trim().trim_matches('"').trim_matches('\'');
    let v = v.strip_prefix('=').unwrap_or(v);
    let v = v.strip_prefix('v').unwrap_or(v);
    v.split('(').next().unwrap_or(v).trim().to_string()
}

/// One dependency's value, or a lockfile's `version`, under `name`: the
/// release it names. `npm:real@1.2.3` names a release of `real`.
fn push_release(name: &str, value: &str, out: &mut Vec<(String, String)>) {
    if let Some(target) = value.strip_prefix("npm:") {
        let real = name_of_spec(target);
        if let Some(version) = target.get(real.len() + 1..) {
            out.push((real.to_string(), bare_version(version)));
        }
        return;
    }
    out.push((name.to_string(), bare_version(value)));
}

fn json_releases(root: &Json) -> Result<Vec<(String, String)>, String> {
    if !matches!(root, Json::Obj(_)) {
        return Err("the document is not an object".into());
    }
    let mut out = Vec::new();
    // The package itself: a copy of a hijacked release is that release.
    if let (Some(name), Some(version)) = (
        root.get("name").and_then(Json::as_str),
        root.get("version").and_then(Json::as_str),
    ) {
        out.push((name.to_string(), bare_version(version)));
    }
    for key in DEPENDENCY_KEYS {
        let Some(map) = root.get(key) else { continue };
        if *key == "dependencies" && map.members().iter().any(|(_, v)| matches!(v, Json::Obj(_))) {
            lockfile_v1_releases(map, &mut out);
        } else {
            for (name, value) in map.members() {
                if let Some(value) = value.as_str() {
                    push_release(name, value, &mut out);
                }
            }
        }
    }
    // package-lock.json v2 and v3: each installed path with its version. An
    // aliased one carries the real name in "name".
    if let Some(packages) = root.get("packages") {
        for (path, entry) in packages.members() {
            let Some(version) = entry.get("version").and_then(Json::as_str) else {
                continue;
            };
            let name = entry
                .get("name")
                .and_then(Json::as_str)
                .or_else(|| path.rsplit_once("node_modules/").map(|(_, n)| n));
            if let Some(name) = name {
                push_release(name, version, &mut out);
            }
        }
    }
    Ok(out)
}

/// package-lock.json v1: a name, its `version`, and its own nested
/// `dependencies`. Bounded by the parse.
fn lockfile_v1_releases(map: &Json, out: &mut Vec<(String, String)>) {
    for (name, entry) in map.members() {
        if let Some(version) = entry.get("version").and_then(Json::as_str) {
            push_release(name, version, out);
        }
        if let Some(nested) = entry.get("dependencies") {
            lockfile_v1_releases(nested, out);
        }
    }
}

/// yarn.lock: the names of an entry's header, then its `version` line
/// (`version "1.2.3"` in v1, `version: 1.2.3` in berry). An alias header
/// installs the package it points at. berry's `resolution:` names the real
/// package and release too.
fn yarn_releases(text: &str) -> Result<Vec<(String, String)>, String> {
    // The same reading as for names, so a file is refused for one exactly
    // when it is refused for the other.
    yarn_names(text)?;
    let mut out = Vec::new();
    let mut current: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        if !(line.starts_with(' ') || line.starts_with('\t')) {
            current.clear();
            let header = line.strip_suffix(':').unwrap_or(line);
            if header == "__metadata" {
                continue;
            }
            for spec in header.split(',') {
                let spec = spec.trim().trim_matches('"');
                let name = name_of_spec(spec);
                let installed = spec
                    .get(name.len() + 1..)
                    .and_then(alias_target)
                    .unwrap_or(name);
                current.push(installed.to_string());
            }
            continue;
        }
        let field = line.trim_start();
        if let Some(rest) = field
            .strip_prefix("version ")
            .or_else(|| field.strip_prefix("version:"))
        {
            for name in &current {
                out.push((name.clone(), bare_version(rest)));
            }
        } else if let Some(rest) = field.strip_prefix("resolution:") {
            let spec = rest.trim().trim_matches('"');
            let name = name_of_spec(spec);
            if let Some(version) = spec.get(name.len() + 1..) {
                let version = version.strip_prefix("npm:").unwrap_or(version);
                out.push((name.to_string(), bare_version(version)));
            }
        }
    }
    Ok(out)
}

/// pnpm-lock.yaml: the release in each key under `packages:` or
/// `snapshots:`, `/a@1.0.0(peer@1)` (v6), `a@1.0.0` (v9) or `/a/1.0.0_peer`
/// (v5).
fn pnpm_releases(text: &str) -> Result<Vec<(String, String)>, String> {
    pnpm_names(text)?;
    // v5 writes /name/version_peer; later versions /name@version(peer).
    let v5 = text.lines().any(|l| {
        l.strip_prefix("lockfileVersion:").is_some_and(|v| {
            v.trim()
                .trim_matches('\'')
                .trim_matches('"')
                .starts_with('5')
        })
    });
    let mut out = Vec::new();
    let mut section = "";
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        let content = line.trim_start_matches(' ');
        if content.is_empty() || content.starts_with('#') {
            continue;
        }
        let indent = line.len() - content.len();
        let Some(key) = yaml_key(content) else {
            continue;
        };
        if indent == 0 {
            section = key;
            continue;
        }
        if !(matches!(section, "packages" | "snapshots") && indent == 2) {
            continue;
        }
        let spec = key.trim_start_matches('/');
        if v5 {
            if let Some((name, version)) = spec.rsplit_once('/') {
                if version.starts_with(|c: char| c.is_ascii_digit()) && !name.is_empty() {
                    let version = version.split('_').next().unwrap_or(version);
                    out.push((name.to_string(), bare_version(version)));
                }
            }
            continue;
        }
        let spec = spec.split('(').next().unwrap_or(spec);
        let name = name_of_spec(spec);
        if let Some(version) = spec.get(name.len() + 1..) {
            out.push((name.to_string(), bare_version(version)));
        }
    }
    Ok(out)
}

/// The package a version specifier names, from `name@range`, `@scope/name@range`
/// or a bare name.
fn name_of_spec(spec: &str) -> &str {
    let from = usize::from(spec.starts_with('@'));
    match spec.get(from..).and_then(|rest| rest.find('@')) {
        Some(at) => spec.get(..from + at).unwrap_or(spec),
        None => spec,
    }
}

/// `npm:evil@^1.0.0` installs `evil` under another name. The name it
/// installs, when `value` is one of those. yarn berry writes every ordinary
/// range as `npm:^1.0.0`, which names no second package: an alias has a name
/// and then an `@`, or is scoped.
fn alias_target(value: &str) -> Option<&str> {
    let target = value.strip_prefix("npm:")?;
    let named = target.starts_with('@') || target.get(1..).is_some_and(|t| t.contains('@'));
    Some(name_of_spec(target)).filter(|n| named && !n.is_empty())
}

// ---------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------

/// A parsed JSON value. Numbers and booleans are not needed, so they are not
/// kept.
#[derive(Debug, PartialEq)]
enum Json {
    Scalar,
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    fn members(&self) -> &[(String, Json)] {
        match self {
            Json::Obj(members) => members,
            _ => &[],
        }
    }
}

/// An array or an object still being read.
enum Open {
    Arr(Vec<Json>),
    /// The members so far, and the key of the value being read.
    Obj(Vec<(String, Json)>, String),
}

struct Reader<'a> {
    text: &'a str,
    at: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.at).copied()
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn fail<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("{what} at byte {}", self.at))
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        self.skip_space();
        if self.peek() == Some(byte) {
            self.at += 1;
            Ok(())
        } else {
            self.fail(&format!("expected '{}'", char::from(byte)))
        }
    }

    /// A string, with the opening quote next.
    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        let mut from = self.at;
        loop {
            match self.peek() {
                None => return self.fail("unterminated string"),
                Some(b'"') => {
                    out.push_str(self.text.get(from..self.at).unwrap_or_default());
                    self.at += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    out.push_str(self.text.get(from..self.at).unwrap_or_default());
                    self.at += 1;
                    let escaped = self.peek();
                    self.at += 1;
                    match escaped {
                        Some(b'"') => out.push('"'),
                        Some(b'\\') => out.push('\\'),
                        Some(b'/') => out.push('/'),
                        Some(b'b') => out.push('\u{8}'),
                        Some(b'f') => out.push('\u{c}'),
                        Some(b'n') => out.push('\n'),
                        Some(b'r') => out.push('\r'),
                        Some(b't') => out.push('\t'),
                        Some(b'u') => out.push(self.unicode()?),
                        _ => return self.fail("invalid escape"),
                    }
                    from = self.at;
                }
                Some(_) => self.at += 1,
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self.text.get(self.at..self.at + 4);
        match digits.and_then(|d| u32::from_str_radix(d, 16).ok()) {
            Some(n) if digits.is_some_and(|d| d.bytes().all(|b| b.is_ascii_hexdigit())) => {
                self.at += 4;
                Ok(n)
            }
            _ => self.fail("invalid \\u escape"),
        }
    }

    /// The character after `\u`, joining a surrogate pair. A lone surrogate
    /// becomes U+FFFD, as a lenient reader would make it.
    fn unicode(&mut self) -> Result<char, String> {
        let first = self.hex4()?;
        if (0xD800..0xDC00).contains(&first) && self.text.get(self.at..self.at + 2) == Some("\\u") {
            let save = self.at;
            self.at += 2;
            let second = self.hex4()?;
            if (0xDC00..0xE000).contains(&second) {
                let joined = 0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00);
                return Ok(char::from_u32(joined).unwrap_or('\u{FFFD}'));
            }
            self.at = save;
        }
        Ok(char::from_u32(first).unwrap_or('\u{FFFD}'))
    }

    /// `"key":`, with the opening quote next after any space.
    fn key(&mut self) -> Result<String, String> {
        self.skip_space();
        let key = self.string()?;
        self.expect(b':')?;
        Ok(key)
    }

    fn literal(&mut self) -> Result<Json, String> {
        for word in ["true", "false", "null"] {
            if self.text.get(self.at..self.at + word.len()) == Some(word) {
                self.at += word.len();
                return Ok(Json::Scalar);
            }
        }
        let start = self.at;
        while matches!(
            self.peek(),
            Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
        ) {
            self.at += 1;
        }
        if self.at == start {
            return self.fail("unexpected character");
        }
        Ok(Json::Scalar)
    }
}

/// Parse one JSON document. Iterative: the nesting is held in a vector, not
/// on the call stack, and refused past [`MAX_DEPTH`].
fn parse_json(text: &str) -> Result<Json, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut r = Reader { text, at: 0 };
    let mut open: Vec<Open> = Vec::new();
    loop {
        // A value starts here.
        r.skip_space();
        let mut value = match r.peek() {
            Some(b'{') => {
                r.at += 1;
                r.skip_space();
                if r.peek() == Some(b'}') {
                    r.at += 1;
                    Json::Obj(Vec::new())
                } else {
                    if open.len() >= MAX_DEPTH {
                        return r.fail("nested too deeply");
                    }
                    let key = r.key()?;
                    open.push(Open::Obj(Vec::new(), key));
                    continue;
                }
            }
            Some(b'[') => {
                r.at += 1;
                r.skip_space();
                if r.peek() == Some(b']') {
                    r.at += 1;
                    Json::Arr(Vec::new())
                } else {
                    if open.len() >= MAX_DEPTH {
                        return r.fail("nested too deeply");
                    }
                    open.push(Open::Arr(Vec::new()));
                    continue;
                }
            }
            Some(b'"') => Json::Str(r.string()?),
            Some(_) => r.literal()?,
            None => return r.fail("unexpected end"),
        };
        // The value is complete: hand it to what holds it, closing every
        // container it completes.
        loop {
            let Some(top) = open.last_mut() else {
                r.skip_space();
                if r.at != r.text.len() {
                    return r.fail("unexpected content after the document");
                }
                return Ok(value);
            };
            r.skip_space();
            let next = r.peek();
            r.at += 1;
            match top {
                Open::Arr(items) => {
                    items.push(value);
                    match next {
                        Some(b',') => break,
                        Some(b']') => {}
                        _ => {
                            r.at -= 1;
                            return r.fail("expected ',' or ']'");
                        }
                    }
                }
                Open::Obj(members, key) => {
                    members.push((std::mem::take(key), value));
                    match next {
                        Some(b',') => {
                            *key = r.key()?;
                            break;
                        }
                        Some(b'}') => {}
                        _ => {
                            r.at -= 1;
                            return r.fail("expected ',' or '}'");
                        }
                    }
                }
            }
            // Closed: the container is now the completed value.
            value = match open.pop() {
                Some(Open::Arr(items)) => Json::Arr(items),
                Some(Open::Obj(members, _)) => Json::Obj(members),
                None => return r.fail("unbalanced"),
            };
        }
    }
}

/// The names one dependency map installs: its keys, and the real package
/// behind an `npm:` alias.
fn dependency_map(map: &Json, out: &mut Vec<String>) {
    for (name, value) in map.members() {
        out.push(name.clone());
        if let Some(target) = value.as_str().and_then(alias_target) {
            out.push(target.to_string());
        }
    }
}

/// package-lock.json v1: `dependencies` maps a name to an entry that may
/// carry `requires` and its own nested `dependencies`. Bounded by the parse.
fn lockfile_v1(map: &Json, out: &mut Vec<String>) {
    for (name, entry) in map.members() {
        out.push(name.clone());
        if let Some(target) = entry
            .get("version")
            .and_then(Json::as_str)
            .and_then(alias_target)
        {
            out.push(target.to_string());
        }
        if let Some(requires) = entry.get("requires") {
            dependency_map(requires, out);
        }
        if let Some(nested) = entry.get("dependencies") {
            lockfile_v1(nested, out);
        }
    }
}

fn json_names(root: &Json) -> Result<Vec<String>, String> {
    if !matches!(root, Json::Obj(_)) {
        return Err("the document is not an object".into());
    }
    let mut out = Vec::new();
    // The package itself. A copy of a campaign package is that package.
    if let Some(name) = root.get("name").and_then(Json::as_str) {
        out.push(name.to_string());
    }
    for key in DEPENDENCY_KEYS {
        let Some(map) = root.get(key) else { continue };
        if *key == "dependencies" && map.members().iter().any(|(_, v)| matches!(v, Json::Obj(_))) {
            // A v1 lockfile, whose entries are objects rather than ranges.
            lockfile_v1(map, &mut out);
        } else {
            dependency_map(map, &mut out);
        }
    }
    for key in ["bundledDependencies", "bundleDependencies"] {
        if let Some(Json::Arr(names)) = root.get(key) {
            out.extend(names.iter().filter_map(Json::as_str).map(str::to_owned));
        }
    }
    // package-lock.json v2 and v3: "node_modules/a/node_modules/@s/b".
    if let Some(packages) = root.get("packages") {
        for (path, entry) in packages.members() {
            if let Some((_, name)) = path.rsplit_once("node_modules/") {
                out.push(name.to_string());
            }
            if let Some(name) = entry.get("name").and_then(Json::as_str) {
                out.push(name.to_string());
            }
            for key in DEPENDENCY_KEYS {
                if let Some(map) = entry.get(key) {
                    dependency_map(map, &mut out);
                }
            }
        }
    }
    out.retain(|n| !n.is_empty());
    Ok(out)
}

// ---------------------------------------------------------------------------
// yarn.lock
// ---------------------------------------------------------------------------

/// v1 headers are `a@^1.0.0, "@s/b@~2.0.0":`; berry quotes the whole header,
/// `"a@npm:^1.0.0, a@npm:1.0.1":`. Either way each spec names a package, and
/// an alias spec names a second one.
fn yarn_names(text: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            // berry records what an alias resolved to.
            if let Some(rest) = line.trim_start().strip_prefix("resolution:") {
                let spec = rest.trim().trim_matches('"');
                out.push(name_of_spec(spec).to_string());
            }
            continue;
        }
        let Some(header) = line.strip_suffix(':') else {
            return Err(format!("line {} is not an entry header", number + 1));
        };
        if header == "__metadata" {
            continue;
        }
        for spec in header.split(',') {
            let spec = spec.trim().trim_matches('"');
            let name = name_of_spec(spec);
            if name.is_empty() {
                return Err(format!("line {} names no package", number + 1));
            }
            out.push(name.to_string());
            if let Some(target) = spec.get(name.len() + 1..).and_then(alias_target) {
                out.push(target.to_string());
            }
        }
    }
    out.retain(|n| !n.is_empty());
    Ok(out)
}

// ---------------------------------------------------------------------------
// pnpm-lock.yaml
// ---------------------------------------------------------------------------

/// A key under `packages:` or `snapshots:`: `/a@1.0.0`, `/@s/b@1.0.0(peer@1)`
/// (v6), `a@1.0.0` (v9) or `/a/1.0.0` (v5).
fn pnpm_key(key: &str) -> &str {
    let key = key.trim_start_matches('/');
    let key = key.split('(').next().unwrap_or(key);
    let name = name_of_spec(key);
    if name.len() < key.len() {
        return name;
    }
    // v5 separates the version with a slash.
    match key.rsplit_once('/') {
        Some((name, version))
            if version.starts_with(|c: char| c.is_ascii_digit()) && !name.is_empty() =>
        {
            name
        }
        _ => key,
    }
}

fn unquote(s: &str) -> &str {
    s.trim().trim_matches('\'').trim_matches('"')
}

/// The key of a `key:` or `key: value` line, without its indentation.
fn yaml_key(content: &str) -> Option<&str> {
    // A quoted key may hold a colon of its own.
    if let Some(rest) = content
        .strip_prefix('\'')
        .or_else(|| content.strip_prefix('"'))
    {
        let quote = content.chars().next()?;
        let end = rest.find(quote)?;
        return rest.get(..end);
    }
    content
        .find(": ")
        .or_else(|| content.strip_suffix(':').map(str::len))
        .and_then(|end| content.get(..end))
        .map(unquote)
}

fn pnpm_names(text: &str) -> Result<Vec<String>, String> {
    if !text.lines().any(|l| l.starts_with("lockfileVersion:")) {
        return Err("no lockfileVersion".into());
    }
    let mut out = Vec::new();
    let mut section = "";
    // Inside importers: the indentation of the dependency map being read.
    let mut map_indent: Option<usize> = None;
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        let content = line.trim_start_matches(' ');
        if content.is_empty() || content.starts_with('#') {
            continue;
        }
        let indent = line.len() - content.len();
        let Some(key) = yaml_key(content) else {
            continue;
        };
        if indent == 0 {
            section = key;
            // v5 and v6 of a single project list its dependencies at the top.
            map_indent = DEPENDENCY_KEYS.contains(&key).then_some(0);
            continue;
        }
        match section {
            "packages" | "snapshots" if indent == 2 => out.push(pnpm_key(key).to_string()),
            "importers" if indent == 4 => {
                map_indent = DEPENDENCY_KEYS.contains(&key).then_some(4);
            }
            _ => {}
        }
        if let Some(at) = map_indent {
            if indent == at + 2 {
                out.push(key.to_string());
            }
        }
    }
    out.retain(|n| !n.is_empty());
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn names(file: &str, text: &str) -> Vec<String> {
        package_names(file, text).expect("parses")
    }

    #[test]
    fn package_json_lists_every_dependency_map_and_nothing_else() {
        let text = r#"{
          "name": "proj",
          "description": "a fork of evil-pkg, which it does not depend on",
          "scripts": {"evil-in-script": "node x"},
          "dependencies": {"react": "^18", "alias": "npm:@evil/real@^1.0.0"},
          "devDependencies": {"sass-formatter": "1"},
          "optionalDependencies": {"fsevents": "*"},
          "peerDependencies": {"vue": "3"},
          "bundledDependencies": ["bundled"]
        }"#;
        let got = names("package.json", text);
        assert_eq!(
            got,
            [
                "proj",
                "react",
                "alias",
                "@evil/real",
                "sass-formatter",
                "fsevents",
                "vue",
                "bundled"
            ]
        );
        assert!(!got.iter().any(|n| n.contains("evil-pkg")));
    }

    #[test]
    fn a_name_written_in_escape_codes_is_read_as_the_name() {
        let got = names("package.json", r#"{"dependencies":{"\u0065vil":"1"}}"#);
        assert_eq!(got, ["evil"]);
    }

    #[test]
    fn package_lock_v1_v2_and_v3() {
        let v1 = r#"{"name":"p","lockfileVersion":1,"requires":true,"dependencies":{
            "a":{"version":"1.0.0","requires":{"b":"^1"},"dependencies":{"c":{"version":"2.0.0"}}},
            "al":{"version":"npm:real@1.0.0"}}}"#;
        assert_eq!(
            names("package-lock.json", v1),
            ["p", "a", "b", "c", "al", "real"]
        );
        let v3 = r#"{"name":"p","lockfileVersion":3,"packages":{
            "":{"name":"p","dependencies":{"a":"^1"}},
            "node_modules/a":{"version":"1.0.0"},
            "node_modules/a/node_modules/@s/b":{"version":"1.0.0"},
            "node_modules/al":{"name":"real","version":"1.0.0"}}}"#;
        let got = names("package-lock.json", v3);
        for n in ["p", "a", "@s/b", "al", "real"] {
            assert!(got.iter().any(|g| g == n), "{n} in {got:?}");
        }
    }

    #[test]
    fn yarn_v1_and_berry() {
        let v1 = "# yarn lockfile v1\n\n\"@s/a@^1.0.0\", \"@s/a@~1.1.0\":\n  version \"1.1.0\"\n\nb@^2.0.0, al@npm:real@^3:\n  version \"2.0.0\"\n  dependencies:\n    c \"^1\"\n";
        assert_eq!(names("yarn.lock", v1), ["@s/a", "@s/a", "b", "al", "real"]);
        let berry = "__metadata:\n  version: 6\n\n\"left-pad@npm:^1.3.0\":\n  version: 1.3.0\n  resolution: \"left-pad@npm:1.3.0\"\n\n\"proj@workspace:.\":\n  version: 0.0.0-use.local\n";
        assert_eq!(names("yarn.lock", berry), ["left-pad", "left-pad", "proj"]);
        assert!(package_names("yarn.lock", "<<<<<<< HEAD\nb@^1:\n").is_err());
    }

    #[test]
    fn pnpm_v5_v6_and_v9() {
        let v5 = "lockfileVersion: 5.4\n\nspecifiers:\n  a: ^1.0.0\n\ndependencies:\n  a: 1.0.0\n\npackages:\n\n  /a/1.0.0:\n    resolution: {integrity: x}\n  /@s/b/2.0.0:\n    dev: false\n";
        assert_eq!(names("pnpm-lock.yaml", v5), ["a", "a", "@s/b"]);
        let v9 = "lockfileVersion: '9.0'\n\nimporters:\n\n  .:\n    dependencies:\n      a:\n        specifier: ^1.0.0\n        version: 1.0.0\n    devDependencies:\n      '@s/b':\n        specifier: ^2\n        version: 2.0.0(react@18.0.0)\n\npackages:\n\n  a@1.0.0:\n    resolution: {integrity: x}\n\n  '@s/b@2.0.0':\n    resolution: {integrity: x}\n\nsnapshots:\n\n  '@s/b@2.0.0(react@18.0.0)':\n    dependencies:\n      react: 18.0.0\n";
        let got = names("pnpm-lock.yaml", v9);
        assert_eq!(got, ["a", "@s/b", "a", "@s/b", "@s/b"]);
        assert!(!got.iter().any(|n| n == "react" || n == "specifier"));
        assert!(package_names("pnpm-lock.yaml", "packages:\n  a@1: {}\n").is_err());
    }

    fn releases(file: &str, text: &str) -> Vec<(String, String)> {
        package_releases(file, text).expect("parses")
    }

    fn has(got: &[(String, String)], name: &str, version: &str) -> bool {
        got.iter().any(|(n, v)| n == name && v == version)
    }

    #[test]
    fn a_package_json_gives_its_own_release_and_each_value_as_written() {
        let text = r#"{"name":"cdeops","version":"13.5.31","dependencies":{
            "a":"1.0.0","b":"=2.0.0","c":"^3.0.0","al":"npm:@s/real@4.0.0"}}"#;
        let got = releases("package.json", text);
        assert!(has(&got, "cdeops", "13.5.31"));
        assert!(has(&got, "a", "1.0.0"));
        assert!(has(&got, "b", "2.0.0"));
        assert!(has(&got, "@s/real", "4.0.0"));
        // A range is kept as written, so it never equals a release.
        assert!(has(&got, "c", "^3.0.0"));
        assert!(!got.iter().any(|(n, _)| n == "al"));
    }

    #[test]
    fn package_lock_v1_v2_and_v3_give_what_they_resolved() {
        let v1 = r#"{"name":"p","lockfileVersion":1,"dependencies":{
            "a":{"version":"0.0.16-129","requires":{"b":"^1"},"dependencies":{"c":{"version":"2.0.0"}}},
            "al":{"version":"npm:real@1.0.0"}}}"#;
        let got = releases("package-lock.json", v1);
        assert!(has(&got, "a", "0.0.16-129"));
        assert!(has(&got, "c", "2.0.0"));
        assert!(has(&got, "real", "1.0.0"));
        assert!(!got.iter().any(|(n, _)| n == "b"));
        let v3 = r#"{"name":"p","lockfileVersion":3,"packages":{
            "":{"name":"p","version":"1.0.0","dependencies":{"a":"^1"}},
            "node_modules/a":{"version":"1.0.0"},
            "node_modules/a/node_modules/@s/b":{"version":"12.0.18-alpha.53"},
            "node_modules/al":{"name":"real","version":"1.0.0"}}}"#;
        let got = releases("package-lock.json", v3);
        assert!(has(&got, "p", "1.0.0"));
        assert!(has(&got, "a", "1.0.0"));
        assert!(has(&got, "@s/b", "12.0.18-alpha.53"));
        assert!(has(&got, "real", "1.0.0"));
    }

    #[test]
    fn yarn_v1_and_berry_give_each_header_its_version() {
        let v1 = "# yarn lockfile v1\n\n\"@s/a@^1.0.0\", \"@s/a@~1.1.0\":\n  version \"1.1.0\"\n\nb@^2.0.0, al@npm:real@^3:\n  version \"2.0.0\"\n  dependencies:\n    c \"^1\"\n";
        let got = releases("yarn.lock", v1);
        assert!(has(&got, "@s/a", "1.1.0"));
        assert!(has(&got, "b", "2.0.0"));
        assert!(has(&got, "real", "2.0.0"));
        assert!(!got.iter().any(|(n, _)| n == "c" || n == "al"));
        let berry = "__metadata:\n  version: 6\n\n\"left-pad@npm:^1.3.0\":\n  version: 1.3.0\n  resolution: \"left-pad@npm:1.3.0\"\n";
        let got = releases("yarn.lock", berry);
        assert_eq!(got, [("left-pad".to_string(), "1.3.0".to_string())]);
        assert!(package_releases("yarn.lock", "<<<<<<< HEAD\nb@^1:\n").is_err());
    }

    #[test]
    fn pnpm_v5_v6_and_v9_give_the_release_in_each_key() {
        let v5 = "lockfileVersion: 5.4\n\npackages:\n\n  /a/1.0.0:\n    resolution: {integrity: x}\n  /@s/b/2.0.0_react@18.0.0:\n    dev: false\n";
        let got = releases("pnpm-lock.yaml", v5);
        assert!(has(&got, "a", "1.0.0"));
        assert!(has(&got, "@s/b", "2.0.0"));
        let v6 = "lockfileVersion: '6.0'\n\npackages:\n\n  /cdeops@13.5.31:\n    resolution: {integrity: x}\n  /@s/b@2.0.0(react@18.0.0):\n    dev: false\n";
        let got = releases("pnpm-lock.yaml", v6);
        assert!(has(&got, "cdeops", "13.5.31"));
        assert!(has(&got, "@s/b", "2.0.0"));
        let v9 = "lockfileVersion: '9.0'\n\npackages:\n\n  a@1.0.0:\n    resolution: {integrity: x}\n\n  '@s/b@2.0.0':\n    resolution: {integrity: x}\n\nsnapshots:\n\n  '@s/b@2.0.0(react@18.0.0)':\n    dependencies:\n      react: 18.0.0\n";
        let got = releases("pnpm-lock.yaml", v9);
        assert_eq!(
            got,
            [
                ("@s/b".to_string(), "2.0.0".to_string()),
                ("a".to_string(), "1.0.0".to_string())
            ]
        );
        assert!(package_releases("pnpm-lock.yaml", "packages:\n  a@1: {}\n").is_err());
    }

    #[test]
    fn hostile_json_is_refused_not_overflowed() {
        let deep = "[".repeat(100_000);
        assert!(parse_json(&deep).is_err());
        let deep_obj = "{\"a\":".repeat(100_000);
        assert!(parse_json(&deep_obj).is_err());
        // Just inside the limit parses.
        let ok = format!("{}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
        assert!(parse_json(&ok).is_ok());
        for bad in [
            "",
            "{",
            "{\"a\":1,}",
            "{\"a\" 1}",
            "[1 2]",
            "{\"a\":\"\\q\"}",
            "{} {}",
            "{\"a\":\"unterminated}",
        ] {
            assert!(parse_json(bad).is_err(), "{bad:?}");
        }
        assert!(package_names("package.json", "[1,2]").is_err());
    }

    #[test]
    fn json_strings_decode_escapes_and_pairs() {
        let v = parse_json(r#"{"k":"a\"b\\c\n\ud83d\ude00\u00e9"}"#).expect("parses");
        assert_eq!(
            v.get("k").and_then(Json::as_str),
            Some("a\"b\\c\n\u{1F600}\u{e9}")
        );
    }

    #[test]
    fn spec_names() {
        assert_eq!(name_of_spec("a@^1"), "a");
        assert_eq!(name_of_spec("@s/a@^1"), "@s/a");
        assert_eq!(name_of_spec("@s/a"), "@s/a");
        assert_eq!(name_of_spec("a"), "a");
        assert_eq!(pnpm_key("/@s/a@1.0.0(b@2)"), "@s/a");
        assert_eq!(pnpm_key("/a/1.0.0"), "a");
        assert_eq!(pnpm_key("/@s/a/1.0.0"), "@s/a");
    }
}
