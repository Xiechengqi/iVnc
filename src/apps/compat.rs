//! Conservative Electron detection and per-launch argument adaptation. The
//! saved shell command is never rewritten. Inspection never executes an app.
use super::{app::ManagedApp, error::ApiError};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{Read, Seek},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

pub const RETRY_MARKER: &str = "[iVnc] Retrying Electron with launch compatibility";
const MAX_BINARY: u64 = 512 * 1024 * 1024;
const MAX_COMMAND: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Compatibility {
    #[default]
    Auto,
    Electron,
    Raw,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sandbox {
    #[default]
    Auto,
    Preserve,
    Disabled,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LaunchOptions {
    pub compatibility: Compatibility,
    pub sandbox: Sandbox,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Electron {
    pub version: Option<String>,
    pub wayland: bool,
    pub ime: bool,
    pub text_input_v3: bool,
}

#[derive(Clone, Debug)]
struct Word {
    value: String,
    start: usize,
    end: usize,
}

/// Accept literal words only. Shell expansion, redirection and compound
/// commands stay with the original shell. Single-quoted metacharacters and
/// escaped spaces are ordinary literal argument data.
fn words(command: &str) -> Option<Vec<Word>> {
    if command.len() > MAX_COMMAND || command.contains(['\0', '\n', '\r']) {
        return None;
    }
    let mut result = Vec::new();
    let mut chars = command.char_indices().peekable();
    while let Some((start, ch)) = chars.peek().copied() {
        if ch == ' ' || ch == '\t' {
            chars.next();
            continue;
        }
        let mut value = String::new();
        let mut quote = None;
        let mut end = command.len();
        while let Some((i, ch)) = chars.next() {
            match (quote, ch) {
                (Some('\''), '\'') | (Some('"'), '"') => quote = None,
                (Some('\''), _) => value.push(ch),
                (Some('"'), '$' | '`') => return None,
                (Some('"'), '\\') => {
                    let (_, next) = chars.next()?;
                    if !matches!(next, '$' | '`' | '"' | '\\') {
                        value.push('\\');
                    }
                    value.push(next);
                }
                (Some('"'), _) => value.push(ch),
                (None, '\'' | '"') => quote = Some(ch),
                (None, '\\') => value.push(chars.next()?.1),
                (None, ' ' | '\t') => {
                    end = i;
                    break;
                }
                (
                    None,
                    '$' | '`' | '~' | '*' | '?' | '[' | ']' | '{' | '}' | ';' | '&' | '|' | '<'
                    | '>' | '(' | ')' | '#',
                ) => return None,
                (None, _) => value.push(ch),
                _ => return None,
            }
        }
        if quote.is_some() {
            return None;
        }
        result.push(Word { value, start, end });
    }
    (!result.is_empty()).then_some(result)
}

fn assignment(word: &str) -> Option<(&str, &str)> {
    let (key, value) = word.split_once('=')?;
    let mut bytes = key.bytes();
    if !bytes
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        || !bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return None;
    }
    Some((key, value))
}

#[derive(Clone)]
struct Target {
    words: Vec<Word>,
    index: usize,
    path: PathBuf,
    env: BTreeMap<OsString, OsString>,
}

fn executable(program: &str, env: &BTreeMap<OsString, OsString>, cwd: &Path) -> Option<PathBuf> {
    let paths: Vec<PathBuf> = if program.contains('/') {
        vec![cwd.join(program)]
    } else {
        // Match the Linux shell's default search path when PATH is unset.
        std::env::split_paths(
            env.get(OsStr::new("PATH"))
                .map(OsString::as_os_str)
                .unwrap_or(OsStr::new("/usr/bin:/bin")),
        )
        .map(|p| cwd.join(p).join(program))
        .collect()
    };
    paths.into_iter().find_map(|p| {
        let meta = fs::metadata(&p).ok()?;
        (meta.is_file() && super::launch::accessible(&p, libc::X_OK))
            .then(|| fs::canonicalize(p).ok())
            .flatten()
    })
}

fn target(command: &str, env: &BTreeMap<OsString, OsString>, cwd: &Path) -> Option<Target> {
    let words = words(command)?;
    let mut env = env.clone();
    let mut i = 0;
    while let Some((key, value)) = words.get(i).and_then(|w| {
        let (key, value) = assignment(&w.value)?;
        command[w.start..w.end]
            .starts_with(&format!("{key}="))
            .then_some((key, value))
    }) {
        env.insert(key.into(), value.into());
        i += 1;
    }
    if words.get(i)?.value == "exec" {
        i += 1;
    }
    if words.get(i)?.value == "env" || words.get(i)?.value == "/usr/bin/env" {
        if executable(&words[i].value, &env, cwd)? != fs::canonicalize("/usr/bin/env").ok()? {
            return None;
        }
        i += 1;
        if words.get(i)?.value == "--" {
            i += 1;
        }
        while let Some((key, value)) = words.get(i).and_then(|w| assignment(&w.value)) {
            env.insert(key.into(), value.into());
            i += 1;
        }
    }
    if words.get(i)?.value == "dbus-run-session"
        || words.get(i)?.value == "/usr/bin/dbus-run-session"
    {
        if executable(&words[i].value, &env, cwd)?
            != fs::canonicalize("/usr/bin/dbus-run-session").ok()?
        {
            return None;
        }
        i += 1;
        if words.get(i)?.value == "--" {
            i += 1;
        }
    }
    let path = executable(&words.get(i)?.value, &env, cwd)?;
    Some(Target {
        words,
        index: i,
        path,
        env,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Fingerprint(PathBuf, u64, u64, u64, i64, i64, i64, i64);
fn fingerprint(path: &Path, m: &fs::Metadata) -> Fingerprint {
    Fingerprint(
        path.into(),
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    )
}
type Cache = VecDeque<(Fingerprint, Option<Electron>)>;
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

fn inspect_binary(path: &Path) -> Option<Electron> {
    let root = path.parent()?;
    if !root.join("resources.pak").is_file() || !root.join("icudtl.dat").is_file() {
        return None;
    }
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > MAX_BINARY {
        return None;
    }
    let key = fingerprint(path, &meta);
    let cache = CACHE.get_or_init(Default::default);
    if let Some((_, found)) = cache.lock().unwrap().iter().find(|(k, _)| *k == key) {
        return found.clone();
    }
    let found = scan_binary(&mut file);
    // Never cache an inspection of a file changed while it was being read.
    if !file.metadata().is_ok_and(|m| fingerprint(path, &m) == key) {
        return None;
    }
    let mut cache = cache.lock().unwrap();
    if cache.len() >= 64 {
        cache.pop_front();
    }
    cache.push_back((key, found.clone()));
    found
}

fn scan_binary(file: &mut File) -> Option<Electron> {
    let mut magic = [0; 4];
    file.read_exact(&mut magic).ok()?;
    if &magic != b"\x7fELF" {
        return None;
    }
    let needles: &[&[u8]] = &[
        b"electron_main_delegate.cc",
        b"ELECTRON_RUN_AS_NODE",
        b"ELECTRON_NO_ASAR",
        b"ozone-platform",
        b"enable-wayland-ime",
        b"wayland-text-input-version",
    ];
    let mut seen = [false; 6];
    let mut version = None;
    let mut buffer = vec![0; 1024 * 1024 + 128];
    let mut overlap = 0;
    let mut total = 4u64;
    let started = Instant::now();
    loop {
        let n = file.read(&mut buffer[overlap..]).ok()?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > MAX_BINARY {
            return None;
        }
        let data = &buffer[..overlap + n];
        for (i, needle) in needles.iter().enumerate() {
            if !seen[i] {
                seen[i] = memchr::memmem::find(data, needle).is_some();
            }
        }
        if version.is_none() {
            for offset in memchr::memmem::find_iter(data, b"Electron/") {
                let suffix = &data[offset + 9..];
                let len = suffix
                    .iter()
                    .take(40)
                    .take_while(|c| c.is_ascii_digit() || **c == b'.')
                    .count();
                let text = std::str::from_utf8(&suffix[..len]).ok()?;
                if text.split('.').count() == 3
                    && text.split('.').all(|s| !s.is_empty())
                    && suffix.get(len).is_some()
                {
                    version = Some(text.to_owned());
                    break;
                }
            }
        }
        if seen.iter().all(|v| *v) && version.is_some() {
            break;
        }
        if started.elapsed() > Duration::from_secs(3) {
            return None;
        }
        let len = data.len();
        overlap = len.min(128);
        buffer.copy_within(len - overlap..len, 0);
    }
    (seen[0] && (seen[1] || seen[2]) && version.is_some()).then_some(Electron {
        version,
        wayland: seen[3],
        ime: seen[4],
        text_input_v3: seen[5],
    })
}

/// Recognize only a shell wrapper whose entire executable body is a literal
/// `exec ... "$@"`. Arbitrary setup, argument rewriting and interpreter switches
/// make it opaque, even when its filename sounds like an Electron app.
fn inspect(
    path: &Path,
    env: &BTreeMap<OsString, OsString>,
    cwd: &Path,
    depth: usize,
) -> Option<Electron> {
    if let Some(info) = inspect_binary(path) {
        return Some(info);
    }
    if depth >= 3 {
        return None;
    }
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.by_ref().take(32769).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 32768 {
        return None;
    }
    let text = std::str::from_utf8(&bytes).ok()?;
    let mut lines = text.lines();
    if !matches!(
        lines.next()?,
        "#!/bin/sh" | "#!/bin/bash" | "#!/usr/bin/env bash" | "#!/usr/bin/env sh"
    ) {
        return None;
    }
    let lines: Vec<_> = lines
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .collect();
    if lines.len() != 1 {
        return None;
    }
    let body = lines[0].strip_prefix("exec ")?.strip_suffix("\"$@\"")?;
    // Forwarded arguments must be a separate shell word, not concatenated
    // onto the executable's name.
    if !body.ends_with([' ', '\t']) {
        return None;
    }
    let inner = target(body.trim(), env, cwd)?;
    if inner.index != 0 || inner.words.len() != 1 {
        return None;
    }
    inspect(&inner.path, &inner.env, cwd, depth + 1)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub generation: String,
    #[serde(default)]
    pub window_seen: bool,
    pub original_command: String,
    pub effective_command: String,
    pub options: LaunchOptions,
    pub uid: u32,
    pub executable: Option<PathBuf>,
    pub electron: Option<Electron>,
    pub added_args: Vec<String>,
    /// Machine-readable reason, translated by the console.
    pub reason: String,
    pub retried: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic_log: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_path: Option<PathBuf>,
}

fn switch<'a>(args: &'a [Word], name: &str) -> Option<&'a str> {
    let mut value = None;
    for (i, word) in args.iter().enumerate() {
        if word.value == "--" {
            break;
        }
        if word.value == name {
            value = Some(args.get(i + 1).map_or("", |w| w.value.as_str()));
        }
        if let Some(last) = word
            .value
            .strip_prefix(name)
            .and_then(|v| v.strip_prefix('='))
        {
            value = Some(last);
        }
    }
    value
}

fn adapt(plan: &mut Plan, target: &Target, known: Option<&Electron>) -> Result<(), ApiError> {
    let args = &target.words[target.index + 1..];
    let backend =
        switch(args, "--ozone-platform").or_else(|| switch(args, "--ozone-platform-hint"));
    if backend.is_some_and(|s| s != "wayland" && s != "auto") {
        return Err(ApiError::new("desktop_backend_conflict", "This Electron command explicitly selects a non-Wayland backend. Remove that setting or choose raw execution."));
    }
    if plan.options.sandbox == Sandbox::Preserve && switch(args, "--no-sandbox").is_some() {
        return Err(ApiError::new(
            "desktop_sandbox_conflict",
            "Preserve sandbox conflicts with an explicit --no-sandbox argument.",
        ));
    }
    let mut added = Vec::new();
    if backend.is_none() && known.is_none_or(|e| e.wayland) {
        added.push("--ozone-platform=wayland".to_owned());
    }
    if known.is_some_and(|e| e.ime)
        && switch(args, "--enable-wayland-ime").is_none()
        && switch(args, "--disable-wayland-ime").is_none()
    {
        added.push("--enable-wayland-ime".into());
    }
    if known.is_some_and(|e| e.text_input_v3)
        && switch(args, "--wayland-text-input-version").is_none()
    {
        added.push("--wayland-text-input-version=3".into());
    }
    if (plan.options.sandbox == Sandbox::Disabled
        || (plan.uid == 0 && plan.options.sandbox == Sandbox::Auto))
        && switch(args, "--no-sandbox").is_none()
    {
        added.push("--no-sandbox".into());
    }
    let end = target.words[target.index].end;
    if !added.is_empty() {
        plan.effective_command = format!(
            "{} {}{}",
            &plan.original_command[..end],
            added.join(" "),
            &plan.original_command[end..]
        );
    }
    plan.added_args = added;
    plan.reason = if plan.retried {
        "electron_retry"
    } else if known.is_none() {
        "electron_forced"
    } else {
        "electron"
    }
    .into();
    Ok(())
}

pub fn plan(
    app: &ManagedApp,
    env: &BTreeMap<OsString, OsString>,
    uid: u32,
) -> Result<Plan, ApiError> {
    let original = app
        .exec_command
        .clone()
        .ok_or("Desktop application has no command")?;
    let mut plan = Plan {
        generation: uuid::Uuid::new_v4().to_string(),
        window_seen: false,
        original_command: original.clone(),
        effective_command: original,
        options: app.desktop_launch.clone(),
        uid,
        executable: None,
        electron: None,
        added_args: Vec::new(),
        reason: "raw".into(),
        retried: false,
        diagnostic_log: None,
        report_path: None,
    };
    if plan.options.compatibility == Compatibility::Raw {
        return Ok(plan);
    }
    let cwd = env
        .get(OsStr::new("HOME"))
        .map(Path::new)
        .unwrap_or(Path::new("/"));
    let Some(target) = target(&plan.original_command, env, cwd) else {
        if plan.options.compatibility == Compatibility::Electron {
            return Err(ApiError::new("desktop_command_unsupported", "Automatic Electron arguments require a literal executable and arguments. Use a direct command or raw execution for shell scripts and expressions."));
        }
        plan.reason = "opaque_command".into();
        return Ok(plan);
    };
    // An Electron executable in Node mode is a CLI, not a desktop client.
    if target
        .env
        .get(OsStr::new("ELECTRON_RUN_AS_NODE"))
        .is_some_and(|v| !v.is_empty())
    {
        plan.reason = "node_mode".into();
        return Ok(plan);
    }
    plan.executable = Some(target.path.clone());
    let electron = inspect(&target.path, &target.env, cwd, 0);
    if electron.is_some() || plan.options.compatibility == Compatibility::Electron {
        adapt(&mut plan, &target, electron.as_ref())?;
        plan.electron = electron;
    } else {
        plan.reason = "unrecognized".into();
    }
    Ok(plan)
}

impl Plan {
    pub fn command(&self) -> std::process::Command {
        let mut command = std::process::Command::new("/bin/sh");
        command.args(["-c", &self.effective_command]);
        command
    }

    pub fn write_report(&self) {
        if let Some(path) = &self.report_path {
            let mut report = self.clone();
            report.diagnostic_log = None;
            report.report_path = None;
            if let Ok(bytes) = serde_json::to_vec(&report) {
                if let Err(error) = crate::remote::store::private_write(path, &bytes) {
                    eprintln!("[iVnc] Cannot save launch details: {error}");
                }
            }
        }
    }

    /// Only unmistakable pre-initialization Electron failures permit a second
    /// attempt, after the subreaper has collected the entire first process tree.
    pub fn retry(&mut self, log: &str, ran: Duration) -> bool {
        if self.retried
            || self.options.compatibility != Compatibility::Auto
            || self.reason != "unrecognized"
            || ran > Duration::from_secs(10)
        {
            return false;
        }
        if let Some(path) = &self.report_path {
            // The compositor/monitor records mapped windows against this
            // generation. Never relaunch an app which already showed a window.
            if !read_report_at(path)
                .is_some_and(|p| p.generation == self.generation && !p.window_seen)
            {
                return false;
            }
        }
        let root = log.contains("electron_main_delegate.cc")
            && log.contains("Running as root without --no-sandbox is not supported")
            && self.uid == 0
            && self.options.sandbox == Sandbox::Auto;
        let backend =
            log.contains("electron/shell/") && log.contains("Missing X server or $DISPLAY");
        if !root && !backend {
            return false;
        }
        let env = std::env::vars_os().collect();
        let cwd = std::env::current_dir().unwrap_or_else(|_| "/".into());
        let Some(target) = target(&self.original_command, &env, &cwd) else {
            return false;
        };
        // Opaque scripts and user-switching wrappers must never be guessed at.
        let mut file = match File::open(&target.path) {
            Ok(file) => file,
            Err(_) => return false,
        };
        if !file
            .metadata()
            .is_ok_and(|m| m.is_file() && m.len() <= MAX_BINARY)
        {
            return false;
        }
        let mut magic = [0; 12];
        if file.read_exact(&mut magic).is_err() || &magic[..4] != b"\x7fELF" {
            return false;
        }
        // AppImage has a defined argument forwarding interface. Other opaque
        // executables need their own Electron binary evidence as well as the
        // log signature: an error from `sudo app` must not add flags to sudo.
        let info = if &magic[8..11] == b"AI\x02" {
            None
        } else {
            if file.rewind().is_err() {
                return false;
            }
            let Some(info) = scan_binary(&mut file) else {
                return false;
            };
            Some(info)
        };
        self.retried = true;
        if adapt(self, &target, info.as_ref()).is_err() || self.added_args.is_empty() {
            return false;
        }
        self.electron = info;
        true
    }
}

pub fn report_path(id: &str) -> PathBuf {
    crate::paths::app_log_dir(id).join("launch-info.json")
}

pub fn read_report(id: &str) -> Option<Plan> {
    read_report_at(&report_path(id))
}

fn read_report_at(path: &Path) -> Option<Plan> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > 256 * 1024 {
        return None;
    }
    serde_json::from_reader(file.take(256 * 1024)).ok()
}

/// Called off the render thread after a managed application mapped a window.
pub fn mark_window_seen(id: &str) {
    mark_window_seen_at(&report_path(id));
}

pub fn mark_window_seen_at(path: &Path) {
    if let Some(mut report) = read_report_at(path) {
        if !report.window_seen {
            report.window_seen = true;
            report.report_path = Some(path.to_owned());
            report.write_report();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::app::AppType;
    use std::os::unix::fs::{symlink, PermissionsExt};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("ivnc-compat-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, bytes).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            path
        }
        fn electron(&self) -> PathBuf {
            self.write("resources.pak", b"test");
            self.write("icudtl.dat", b"test");
            self.write("desktop app", b"\x7fELF\0electron_main_delegate.cc\0ELECTRON_RUN_AS_NODE\0ELECTRON_NO_ASAR\0Electron/44.9.3\0ozone-platform\0enable-wayland-ime\0wayland-text-input-version\0")
        }
        fn app(&self, command: &str) -> ManagedApp {
            let mut app = ManagedApp::for_test("compat", AppType::DesktopApp);
            app.exec_command = Some(command.into());
            app
        }
        fn env(&self) -> BTreeMap<OsString, OsString> {
            [
                ("HOME".into(), self.0.clone().into()),
                ("PATH".into(), self.0.clone().into()),
            ]
            .into()
        }
        fn plan(&self, command: &str, uid: u32) -> Plan {
            plan(&self.app(command), &self.env(), uid).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn recognizes_symlinked_electron_without_executing_it() {
        let f = Fixture::new();
        let binary = f.electron();
        symlink(&binary, f.0.join("amp-desktop")).unwrap();
        let p = f.plan("amp-desktop", 0);
        assert_eq!(p.electron.unwrap().version.as_deref(), Some("44.9.3"));
        assert_eq!(p.executable.unwrap(), binary);
        assert_eq!(p.original_command, "amp-desktop");
        assert_eq!(
            p.added_args,
            [
                "--ozone-platform=wayland",
                "--enable-wayland-ime",
                "--wayland-text-input-version=3",
                "--no-sandbox"
            ]
        );
        assert!(p
            .effective_command
            .starts_with("amp-desktop --ozone-platform=wayland"));
    }

    #[test]
    fn names_and_chromium_resources_alone_do_not_identify_electron() {
        let f = Fixture::new();
        f.write("resources.pak", b"test");
        f.write("icudtl.dat", b"test");
        f.write("electron", b"\x7fELF\0Chromium\0ozone-platform\0");
        assert_eq!(
            f.plan("electron --custom", 0).effective_command,
            "electron --custom"
        );
        f.write("amp-desktop", b"#!/bin/sh\ntouch should-not-run\n");
        assert!(f.plan("amp-desktop", 0).added_args.is_empty());
        assert!(!f.0.join("should-not-run").exists());
    }

    #[test]
    fn requires_version_and_layout_in_addition_to_electron_marker_strings() {
        let f = Fixture::new();
        let binary = f.electron();
        fs::remove_file(f.0.join("icudtl.dat")).unwrap();
        assert!(inspect_binary(&binary).is_none());
        f.write("icudtl.dat", b"test");
        f.write(
            "desktop app",
            b"\x7fELF\0electron_main_delegate.cc\0ELECTRON_RUN_AS_NODE\0",
        );
        assert!(inspect_binary(&binary).is_none());
    }

    #[test]
    fn sandbox_policy_uses_effective_uid_and_respects_explicit_choices() {
        let f = Fixture::new();
        f.electron();
        assert!(!f
            .plan("'desktop app'", 1000)
            .added_args
            .iter()
            .any(|a| a == "--no-sandbox"));
        let mut app = f.app("'desktop app'");
        app.desktop_launch.sandbox = Sandbox::Preserve;
        assert!(!plan(&app, &f.env(), 0)
            .unwrap()
            .added_args
            .iter()
            .any(|a| a == "--no-sandbox"));
        app.desktop_launch.sandbox = Sandbox::Disabled;
        assert!(plan(&app, &f.env(), 1000)
            .unwrap()
            .added_args
            .iter()
            .any(|a| a == "--no-sandbox"));
        app.exec_command = Some("'desktop app' --no-sandbox".into());
        app.desktop_launch.sandbox = Sandbox::Preserve;
        assert_eq!(
            plan(&app, &f.env(), 0).unwrap_err().code,
            "desktop_sandbox_conflict"
        );
    }

    #[test]
    fn existing_flags_are_not_duplicated_and_separator_is_not_crossed() {
        let f = Fixture::new();
        f.electron();
        let command = "'desktop app' --ozone-platform wayland --enable-wayland-ime --wayland-text-input-version=1 --no-sandbox -- 'literal --ozone-platform=x11'";
        let p = f.plan(command, 0);
        assert!(p.added_args.is_empty());
        assert_eq!(p.effective_command, command);
        let p = f.plan("'desktop app' -- '--no-sandbox'", 0);
        assert!(p
            .effective_command
            .contains("--no-sandbox -- '--no-sandbox'"));
        let p2 = f.plan(&p.effective_command, 0);
        assert!(p2.added_args.is_empty());
    }

    #[test]
    fn preserves_quoting_and_inline_environment_while_resolving_the_real_path() {
        let f = Fixture::new();
        f.electron();
        let command = format!(
            "PATH='{}' LANG=zh_CN.UTF-8 'desktop app' 'a;$(literal)' \"two words\"",
            f.0.display()
        );
        let p = f.plan(&command, 0);
        assert!(p
            .effective_command
            .ends_with("'a;$(literal)' \"two words\""));
        assert!(p.effective_command.starts_with(&format!(
            "PATH='{}' LANG=zh_CN.UTF-8 'desktop app' --",
            f.0.display()
        )));
        assert!(target("'PATH=/bin' 'desktop app'", &f.env(), &f.0).is_none());
    }

    #[test]
    fn refuses_shell_expansion_and_user_switching_in_auto_mode() {
        let f = Fixture::new();
        f.electron();
        for command in [
            "'desktop app' $ARG",
            "'desktop app' \"$HOME\"",
            "'desktop app' > out",
            "'desktop app'; true",
            "su - ubuntu -c 'desktop app'",
            "sudo 'desktop app'",
            "'desktop app' $(id)",
        ] {
            let p = f.plan(command, 0);
            assert_eq!(p.effective_command, command);
            assert!(p.added_args.is_empty());
        }
    }

    #[test]
    fn transparent_wrappers_forward_but_opaque_scripts_are_not_guessed() {
        let f = Fixture::new();
        let binary = f.electron();
        f.write(
            "wrapper",
            format!(
                "#!/bin/sh\n# A transparent launcher\nexec '{}' \"$@\"\n",
                binary.display()
            )
            .as_bytes(),
        );
        assert_eq!(f.plan("wrapper --custom", 0).reason, "electron");
        f.write(
            "wrapper",
            format!(
                "#!/bin/sh\nexec '{}' --ozone-platform=x11 \"$@\"\n",
                binary.display()
            )
            .as_bytes(),
        );
        assert_eq!(f.plan("wrapper", 0).reason, "unrecognized");
        f.write(
            "wrapper",
            format!("#!/bin/sh\nexec '{}'\"$@\"\n", binary.display()).as_bytes(),
        );
        assert_eq!(f.plan("wrapper", 0).reason, "unrecognized");
        f.write("wrapper", b"#!/bin/sh\nexec wrapper \"$@\"\n");
        assert_eq!(f.plan("wrapper", 0).reason, "unrecognized");
        let command = format!("/usr/bin/env PATH='{}' wrapper", f.0.display());
        assert_eq!(f.plan(&command, 0).reason, "unrecognized");
    }

    #[test]
    fn explicit_backend_choices_are_validated_without_overriding_them() {
        let f = Fixture::new();
        f.electron();
        for flag in [
            "--ozone-platform=x11",
            "--ozone-platform-hint x11",
            "--ozone-platform=wayland --ozone-platform=x11",
        ] {
            assert_eq!(
                plan(&f.app(&format!("'desktop app' {flag}")), &f.env(), 0)
                    .unwrap_err()
                    .code,
                "desktop_backend_conflict"
            );
        }
        let p = f.plan(
            "'desktop app' --ozone-platform=auto --disable-wayland-ime",
            0,
        );
        assert!(!p
            .added_args
            .iter()
            .any(|a| a.starts_with("--ozone-platform") || a == "--enable-wayland-ime"));
        assert!(!f
            .plan(
                "'desktop app' --ozone-platform=x11 --ozone-platform=wayland",
                0
            )
            .added_args
            .iter()
            .any(|a| a.starts_with("--ozone-platform")));
    }

    #[test]
    fn raw_and_electron_node_mode_preserve_the_original_command() {
        let f = Fixture::new();
        f.electron();
        let mut app = f.app("'desktop app' --ozone-platform=x11");
        app.desktop_launch.compatibility = Compatibility::Raw;
        assert_eq!(plan(&app, &f.env(), 0).unwrap().reason, "raw");
        app.desktop_launch.compatibility = Compatibility::Auto;
        let mut env = f.env();
        env.insert("ELECTRON_RUN_AS_NODE".into(), "1".into());
        assert_eq!(plan(&app, &env, 0).unwrap().reason, "node_mode");
    }

    #[test]
    fn forced_electron_is_explicit_and_still_requires_a_literal_command() {
        let f = Fixture::new();
        f.write("custom", b"#!/bin/sh\nexit 0\n");
        let mut app = f.app("custom");
        app.desktop_launch.compatibility = Compatibility::Electron;
        let p = plan(&app, &f.env(), 0).unwrap();
        assert_eq!(p.reason, "electron_forced");
        assert!(p.added_args.iter().any(|s| s == "--no-sandbox"));
        app.exec_command = Some("custom | cat".into());
        assert_eq!(
            plan(&app, &f.env(), 0).unwrap_err().code,
            "desktop_command_unsupported"
        );
    }

    #[test]
    fn binary_replacement_and_symlink_changes_invalidate_detection() {
        let f = Fixture::new();
        let binary = f.electron();
        symlink(&binary, f.0.join("launcher")).unwrap();
        assert!(f.plan("launcher", 0).electron.is_some());
        f.write("other", b"\x7fELF\0not Electron\0");
        fs::remove_file(f.0.join("launcher")).unwrap();
        symlink(f.0.join("other"), f.0.join("launcher")).unwrap();
        assert!(f.plan("launcher", 0).electron.is_none());
        f.write("desktop app", b"\x7fELF\0replaced\0");
        assert!(f.plan("'desktop app'", 0).electron.is_none());
    }

    #[test]
    fn fallback_requires_precise_early_electron_failure_and_runs_only_once() {
        let f = Fixture::new();
        let mut bytes = b"\x7fELF\0\0\0\0AI\x02\0".to_vec();
        bytes.extend_from_slice(b"opaque AppImage fixture");
        f.write("opaque-app", &bytes);
        let command = format!("'{}/opaque-app'", f.0.display());
        let mut p = f.plan(&command, 0);
        assert!(!p.retry("Missing X server or $DISPLAY", Duration::from_secs(1)));
        let log = "electron/shell/app/electron_main_delegate.cc: Running as root without --no-sandbox is not supported";
        assert!(!p.retry(log, Duration::from_secs(20)));
        assert!(p.retry(log, Duration::from_secs(1)));
        assert_eq!(p.reason, "electron_retry");
        assert!(!p.retry(log, Duration::from_secs(1)));
        let mut p = f.plan(&command, 0);
        p.options.sandbox = Sandbox::Preserve;
        assert!(!p.retry(log, Duration::from_secs(1)));
    }

    #[test]
    fn a_mapped_window_or_a_newer_launch_generation_prevents_fallback() {
        let f = Fixture::new();
        f.write("appimage", b"\x7fELF\0\0\0\0AI\x02\0fixture");
        let command = format!("'{}/appimage'", f.0.display());
        let mut p = f.plan(&command, 0);
        let report = f.0.join("launch-info.json");
        p.report_path = Some(report.clone());
        p.write_report();
        mark_window_seen_at(&report);
        assert!(read_report_at(&report).unwrap().window_seen);
        let log = "electron/shell/app/electron_main_delegate.cc: Running as root without --no-sandbox is not supported";
        assert!(!p.retry(log, Duration::from_secs(1)));
        let mut newer = p.clone();
        newer.generation = "newer-generation".into();
        newer.write_report();
        assert!(!p.retry(log, Duration::from_secs(1)));
    }
}
