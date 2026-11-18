//! Chrome's desktop preset: one resolver for previews, persistence and launches.
//! Manual commands never consume these settings. Resource claims outlive edits
//! and stay with the running process until its entire process tree has stopped.
use super::{app::ManagedApp, error::ApiError};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsString,
    fs::{self, File, OpenOptions},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::OpenOptionsExt,
    },
    path::{Component, Path, PathBuf},
    sync::{Mutex, MutexGuard, OnceLock},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommandMode {
    #[default]
    Generated,
    Manual,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ChromeConfig {
    pub mode: CommandMode,
    pub profile_dir: Option<String>,
    pub crash_dir: Option<String>,
    pub debug_port: Option<u16>,
}

impl ChromeConfig {
    pub fn builtin() -> Self {
        Self {
            profile_dir: Some(
                crate::paths::chrome_profile()
                    .to_string_lossy()
                    .into_owned(),
            ),
            crash_dir: Some(
                crate::paths::chrome_crash_dumps_dir()
                    .to_string_lossy()
                    .into_owned(),
            ),
            debug_port: Some(super::api::chrome_devtools_port()),
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolved {
    pub app_root: PathBuf,
    pub profile_dir: PathBuf,
    pub crash_dir: PathBuf,
    pub temp_dir: PathBuf,
    pub debug_port: Option<u16>,
}

pub fn generated(app: &ManagedApp) -> bool {
    app.chrome
        .as_ref()
        .is_some_and(|c| c.mode == CommandMode::Generated)
}

fn invalid(code: &'static str, message: &str, path: &Path) -> ApiError {
    ApiError::new(code, format!("{message}: {}", path.display()))
        .param("path", path.to_string_lossy().into_owned())
}

/// Canonicalize the existing prefix as well as symlink aliases of new paths.
/// Parent traversal is deliberately rejected rather than interpreted through
/// possibly missing or symlinked directories.
pub fn normalize(path: &Path) -> Result<PathBuf, ApiError> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(invalid(
            "chrome_path_invalid",
            "Use an absolute path without '..'",
            path,
        ));
    }
    let mut prefix = path.to_path_buf();
    let mut tail = Vec::new();
    while !prefix
        .try_exists()
        .map_err(|e| invalid("chrome_path_invalid", &e.to_string(), path))?
    {
        if fs::symlink_metadata(&prefix).is_ok() {
            return Err(invalid(
                "chrome_path_invalid",
                "Broken symbolic link",
                &prefix,
            ));
        }
        tail.push(
            prefix
                .file_name()
                .ok_or("Invalid Chrome path")?
                .to_os_string(),
        );
        if !prefix.pop() {
            return Err("Invalid Chrome path".into());
        }
    }
    let mut resolved = fs::canonicalize(&prefix)
        .map_err(|e| invalid("chrome_path_invalid", &e.to_string(), path))?;
    if !resolved.is_dir() {
        return Err(invalid("chrome_path_invalid", "Expected a directory", path));
    }
    for component in tail.into_iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

pub fn resolve(app: &ManagedApp) -> Result<Resolved, ApiError> {
    let config = app.chrome.as_ref().ok_or("Not a Chrome preset")?;
    let root = normalize(&crate::paths::app_dir(&app.id))?;
    let path = |value: &Option<String>, default: &str| {
        let value = value.as_deref().map(str::trim).filter(|s| !s.is_empty());
        let raw = value
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join(default));
        if raw
            .as_os_str()
            .as_encoded_bytes()
            .iter()
            .any(|b| matches!(*b, 0 | b'\n' | b'\r'))
        {
            return Err(invalid(
                "chrome_path_invalid",
                "Path contains control characters",
                &raw,
            ));
        }
        let path = normalize(&raw)?;
        if root.starts_with(&path) {
            return Err(invalid(
                "chrome_path_invalid",
                "Chrome data cannot contain the application root",
                &path,
            ));
        }
        Ok(path)
    };
    if config.debug_port == Some(0) {
        return Err(ApiError::new(
            "chrome_port_invalid",
            "Remote debugging port must be 1–65535 or empty",
        ));
    }
    let resolved = Resolved {
        app_root: root.clone(),
        profile_dir: path(&config.profile_dir, "profile")?,
        crash_dir: path(&config.crash_dir, "crash")?,
        // Chrome creates a native singleton socket below TMPDIR. UUID app
        // directories can already exceed sockaddr_un's 108-byte path limit.
        temp_dir: PathBuf::from(format!(
            "/tmp/ivnc-chrome-{}-{:016x}",
            unsafe { libc::geteuid() },
            xxhash_rust::xxh64::xxh64(root.as_os_str().as_encoded_bytes(), 0)
        )),
        debug_port: config.debug_port,
    };
    let cache = root.join("home/.cache");
    if resolved.profile_dir.starts_with(&resolved.crash_dir)
        || overlap(&resolved.profile_dir, &resolved.temp_dir)
        || overlap(&resolved.crash_dir, &resolved.temp_dir)
        || resolved.profile_dir.starts_with(&cache)
        || root.join("home").starts_with(&resolved.crash_dir)
    {
        return Err(ApiError::new("chrome_path_invalid", "Profile, crash and temporary directories must not contain one another; a crash subdirectory of the profile is allowed"));
    }
    Ok(resolved)
}

pub fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-'))
    {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

pub fn validate_proxy(proxy: Option<&str>) -> Result<(), ApiError> {
    let Some(proxy) = proxy.filter(|s| !s.is_empty()) else {
        return Ok(());
    };
    let valid = reqwest::Url::parse(proxy).is_ok_and(|url| {
        matches!(
            url.scheme(),
            "http" | "https" | "socks4" | "socks5" | "socks5h"
        ) && url.host_str().is_some()
            && url.query().is_none()
            && url.fragment().is_none()
            && matches!(url.path(), "" | "/")
    }) && !proxy.chars().any(char::is_whitespace);
    if !valid {
        return Err(ApiError::new(
            "chrome_proxy_invalid",
            "Enter a valid http, https, socks4, socks5 or socks5h proxy URL",
        ));
    }
    Ok(())
}

pub fn command(app: &ManagedApp, resolved: &Resolved) -> String {
    let mut command = format!(
        "profile_dir={}; crash_dir={}; export TMPDIR={}; export XDG_CONFIG_HOME=\"$HOME/.config\" XDG_CACHE_HOME=\"$HOME/.cache\" XDG_DATA_HOME=\"$HOME/.local/share\"; exec google-chrome --user-data-dir=\"$profile_dir\" --crash-dumps-dir=\"$crash_dir\"",
        shell_quote(&resolved.profile_dir.to_string_lossy()),
        shell_quote(&resolved.crash_dir.to_string_lossy()),
        shell_quote(&resolved.temp_dir.to_string_lossy()),
    );
    if let Some(proxy) = app.proxy_url.as_deref().filter(|s| !s.is_empty()) {
        // Chromium's socks5 already resolves hostnames through the proxy.
        // Keep the existing socks5h setting compatible, using its native name.
        let proxy = if proxy
            .get(..10)
            .is_some_and(|s| s.eq_ignore_ascii_case("socks5h://"))
        {
            format!("socks5://{}", &proxy[10..])
        } else {
            proxy.to_owned()
        };
        command.push_str(&format!(" --proxy-server={}", shell_quote(&proxy)));
    }
    if let Some(port) = resolved.debug_port {
        command.push_str(&format!(
            " --remote-debugging-host=0.0.0.0 --remote-debugging-port={port}"
        ));
    }
    let class: String = app
        .id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    command.push_str(&format!(" --ozone-platform=wayland --class=ivnc-chrome-{class} --test-type --no-first-run --no-default-browser-check --disable-features=MediaRouter --disable-background-networking"));
    if unsafe { libc::geteuid() } == 0 {
        command.push_str(" --no-sandbox --disable-setuid-sandbox");
    }
    command.push_str(" --disable-dev-shm-usage");
    command
}

pub fn refresh_command(app: &mut ManagedApp) -> Result<(), ApiError> {
    if generated(app) {
        validate_proxy(app.proxy_url.as_deref())?;
        app.exec_command = Some(command(app, &resolve(app)?));
    }
    Ok(())
}

pub fn apply_input(app: &mut ManagedApp, body: &serde_json::Value) -> Result<(), ApiError> {
    if let Some(preset) = body.get("desktop_preset") {
        match preset.as_str() {
            Some("chrome") => {
                app.chrome.get_or_insert_default();
            }
            Some("custom") if app.id != super::datadir::BUILTIN_CHROME_ID => {
                app.chrome = None;
            }
            _ => {
                return Err(ApiError::new(
                    "desktop_preset_invalid",
                    "Choose custom or Chrome; built-in Chrome keeps its preset",
                ))
            }
        }
    }
    if let Some(current) = app.chrome.as_ref() {
        if let Some(value) = body.get("chrome_config") {
            let mut merged = serde_json::to_value(current).unwrap();
            let fields = value.as_object().ok_or_else(|| {
                ApiError::new("chrome_config_invalid", "Expected Chrome configuration")
            })?;
            for (key, value) in fields {
                merged[key] = value.clone();
            }
            app.chrome = Some(serde_json::from_value(merged).map_err(|e| {
                ApiError::new(
                    "chrome_config_invalid",
                    format!("Invalid Chrome configuration: {e}"),
                )
            })?);
        }
    } else if body.get("chrome_config").is_some_and(|v| !v.is_null()) {
        return Err(ApiError::new(
            "desktop_preset_invalid",
            "Select Chrome before setting Chrome configuration",
        ));
    }
    if body.get("proxy_url").is_some() {
        if !body["proxy_url"].is_null() && !body["proxy_url"].is_string() {
            return Err(ApiError::new(
                "chrome_proxy_invalid",
                "Expected a proxy URL or null",
            ));
        }
        app.proxy_url = body["proxy_url"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
    }
    if generated(app) {
        refresh_command(app)?;
    } else if let Some(value) = body.get("exec_command") {
        app.exec_command = value
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .map(str::to_owned);
    }
    if app
        .exec_command
        .as_ref()
        .is_none_or(|s| s.trim().is_empty())
    {
        return Err(ApiError::new(
            "exec_command_required",
            "Enter a launch command",
        ));
    }
    Ok(())
}

fn overlap(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

fn conflict(a: &Resolved, b: &Resolved, name: &str) -> Result<(), ApiError> {
    for first in [&a.profile_dir, &a.crash_dir, &a.temp_dir] {
        for second in [&b.profile_dir, &b.crash_dir, &b.temp_dir] {
            if overlap(first, second) {
                return Err(ApiError::new(
                    "chrome_directory_conflict",
                    "Chrome data directories overlap another application",
                )
                .param("name", name)
                .param("path", first.to_string_lossy().into_owned()));
            }
        }
    }
    if a.debug_port.is_some() && a.debug_port == b.debug_port {
        return Err(ApiError::new(
            "chrome_port_conflict",
            "Remote debugging port is assigned to another application",
        )
        .param("name", name)
        .param("port", a.debug_port));
    }
    Ok(())
}

type Claims = HashMap<PathBuf, (String, Resolved)>;
static CLAIMS: OnceLock<Mutex<Claims>> = OnceLock::new();
fn claims() -> MutexGuard<'static, Claims> {
    CLAIMS.get_or_init(Default::default).lock().unwrap()
}

/// Keep this guard until the database write/cleanup is complete. Launches use
/// the same lock, closing the check/commit race against active resource claims.
#[derive(Debug)]
pub struct ResourceLock {
    _guard: MutexGuard<'static, Claims>,
}

pub fn configuration_lock(
    app: &ManagedApp,
    peers: &[ManagedApp],
) -> Result<ResourceLock, ApiError> {
    let guard = claims();
    if generated(app) {
        validate_proxy(app.proxy_url.as_deref())?;
        let resolved = resolve(app)?;
        validate_peers(app, &resolved, peers, &guard)?;
    }
    Ok(ResourceLock { _guard: guard })
}

fn validate_peers(
    app: &ManagedApp,
    resolved: &Resolved,
    peers: &[ManagedApp],
    active: &Claims,
) -> Result<(), ApiError> {
    for peer in peers.iter().filter(|p| p.id != app.id) {
        let root = normalize(&crate::paths::app_dir(&peer.id))?;
        if [&resolved.profile_dir, &resolved.crash_dir]
            .iter()
            .any(|path| overlap(path, &root))
        {
            return Err(ApiError::new(
                "chrome_directory_conflict",
                "Chrome data overlaps another application's directory",
            )
            .param("name", peer.name.clone()));
        }
        if generated(peer) {
            conflict(resolved, &resolve(peer)?, &peer.name)?;
        }
    }
    for path in [&resolved.profile_dir, &resolved.crash_dir] {
        if fs::symlink_metadata(path.join(OWNER)).is_ok() && !owned(path, &resolved.app_root) {
            return Err(invalid(
                "chrome_directory_conflict",
                "Directory belongs to another application",
                path,
            ));
        }
    }
    for (root, (name, other)) in active {
        if *root != resolved.app_root {
            conflict(resolved, other, name)?;
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct Lease {
    root: PathBuf,
    _file: File,
}
impl Drop for Lease {
    fn drop(&mut self) {
        let mut guard = claims();
        if let Some((_, resolved)) = guard.remove(&self.root) {
            if private_temp(&resolved.temp_dir) && owned(&resolved.temp_dir, &self.root) {
                let _ = fs::remove_dir_all(&resolved.temp_dir);
            }
        }
    }
}

fn private_temp(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    fs::symlink_metadata(path).is_ok_and(|m| {
        m.is_dir() && m.uid() == unsafe { libc::geteuid() } && m.mode() & 0o777 == 0o700
    })
}

const OWNER: &str = ".ivnc-chrome-owner";
fn owned(path: &Path, root: &Path) -> bool {
    fs::read_to_string(path.join(OWNER)).is_ok_and(|s| s == root.to_string_lossy())
}

fn claim_directory(path: &Path, root: &Path) -> Result<(), ApiError> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|e| invalid("chrome_directory_unavailable", &e.to_string(), path))?;
    if !super::launch::accessible(path, libc::R_OK | libc::W_OK | libc::X_OK) {
        return Err(invalid(
            "chrome_directory_unavailable",
            "Chrome directory is not writable",
            path,
        ));
    }
    let marker = path.join(OWNER);
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&marker)
    {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(root.to_string_lossy().as_bytes())
                .map_err(|e| e.to_string())?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && owned(path, root) => {}
        Err(_) => {
            return Err(invalid(
                "chrome_directory_conflict",
                "Directory belongs to another application",
                path,
            ))
        }
    }
    Ok(())
}

/// Chrome's native SingletonLock identifies the host and browser PID. Never
/// force a live (or remotely owned/unknown) profile open by removing its lock.
pub fn check_singleton(profile: &Path) -> Result<(), ApiError> {
    let lock = profile.join("SingletonLock");
    let target = match fs::read_link(&lock) {
        Ok(target) => target,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {
            return Err(invalid(
                "chrome_profile_busy",
                "Unrecognized Chrome profile lock",
                &lock,
            ))
        }
    };
    let target = target.to_string_lossy();
    let (host, pid) = target.rsplit_once('-').ok_or_else(|| {
        invalid(
            "chrome_profile_busy",
            "Unrecognized Chrome profile lock",
            &lock,
        )
    })?;
    let hostname = fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default();
    let pid = pid.parse::<i32>().ok().filter(|p| *p > 0);
    if hostname.trim() != host
        || pid.is_none()
        || unsafe { libc::kill(pid.unwrap_or(0), 0) } == 0
        || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    {
        return Err(invalid(
            "chrome_profile_busy",
            "Chrome profile is already in use",
            profile,
        ));
    }
    // Let Chrome recover its own confirmed-stale native singleton artifacts.
    Ok(())
}

pub fn acquire(app: &mut ManagedApp, peers: &[ManagedApp]) -> Result<Option<Lease>, ApiError> {
    if !generated(app) {
        return Ok(None);
    }
    let installation = installation_for_app(app);
    if matches!(installation.status, "missing" | "not_executable") {
        return Err(ApiError::new(
            "chrome_binary_unavailable",
            "google-chrome is not installed or executable in the application's PATH",
        )
        .param("status", installation.status));
    }
    let mut guard = claims();
    let resolved = resolve(app)?;
    validate_proxy(app.proxy_url.as_deref())?;
    validate_peers(app, &resolved, peers, &guard)?;
    if let Some(port) = resolved.debug_port {
        let socket =
            std::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port)).map_err(|_| {
                ApiError::new(
                    "chrome_port_in_use",
                    "Remote debugging port is already in use",
                )
                .param("port", port)
            })?;
        drop(socket);
    }
    check_singleton(&resolved.profile_dir)?;
    claim_directory(&resolved.profile_dir, &resolved.app_root)?;
    claim_directory(&resolved.crash_dir, &resolved.app_root)?;
    claim_directory(&resolved.temp_dir, &resolved.app_root)?;
    if !private_temp(&resolved.temp_dir) {
        return Err(invalid(
            "chrome_directory_unavailable",
            "Chrome temporary directory must be private and owned by the application user",
            &resolved.temp_dir,
        ));
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(resolved.profile_dir.join(".ivnc-profile.lock"))
        .map_err(|e| e.to_string())?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(invalid(
            "chrome_profile_busy",
            "Chrome profile is already in use",
            &resolved.profile_dir,
        ));
    }
    app.exec_command = Some(command(app, &resolved));
    // Freeze absolute manager paths for a worker, whose IVNC_HOME is private.
    let config = app.chrome.as_mut().unwrap();
    config.profile_dir = Some(resolved.profile_dir.to_string_lossy().into_owned());
    config.crash_dir = Some(resolved.crash_dir.to_string_lossy().into_owned());
    app.env_vars.get_or_insert_default().insert(
        "TMPDIR".into(),
        resolved.temp_dir.to_string_lossy().into_owned(),
    );
    guard.insert(
        resolved.app_root.clone(),
        (app.name.clone(), resolved.clone()),
    );
    Ok(Some(Lease {
        root: resolved.app_root,
        _file: file,
    }))
}

/// The process manager holds its operation lock while this runs, so watchdog
/// restarts cannot race deletion. Other app launches also take the claim lock.
pub fn cleanup_lock(app: &ManagedApp) -> Result<ResourceLock, ApiError> {
    let guard = claims();
    if !generated(app) {
        return Err(ApiError::new(
            "chrome_manual_cleanup",
            "Switch to configuration mode to manage Chrome data directories",
        ));
    }
    let resolved = resolve(app)?;
    if guard.contains_key(&resolved.app_root) {
        return Err(ApiError::new(
            "chrome_profile_busy",
            "Stop Chrome before clearing its data",
        ));
    }
    for (name, other) in guard.values() {
        conflict(&resolved, other, name)?;
    }
    check_singleton(&resolved.profile_dir)?;
    for path in [
        &resolved.profile_dir,
        &resolved.crash_dir,
        &resolved.temp_dir,
    ] {
        if path.exists()
            && !path.starts_with(&resolved.app_root)
            && !owned(path, &resolved.app_root)
        {
            return Err(invalid(
                "chrome_directory_unmanaged",
                "External Chrome directory has not been claimed by this application",
                path,
            ));
        }
    }
    if resolved.temp_dir.exists() && !private_temp(&resolved.temp_dir) {
        return Err(invalid(
            "chrome_directory_unmanaged",
            "Chrome temporary directory is not private",
            &resolved.temp_dir,
        ));
    }
    Ok(ResourceLock { _guard: guard })
}

/// Deleting an application removes its managed root, never its configured
/// external directories. An unclaimed or unavailable external profile must
/// therefore not make an otherwise stopped application impossible to delete.
pub fn managed_root_lock(app: &ManagedApp) -> Result<ResourceLock, ApiError> {
    let guard = claims();
    let root = normalize(&crate::paths::app_dir(&app.id))?;
    for (name, active) in guard.values() {
        if [&active.profile_dir, &active.crash_dir, &active.temp_dir]
            .iter()
            .any(|path| overlap(&root, path))
            || root == active.app_root
        {
            return Err(ApiError::new(
                "chrome_profile_busy",
                "Application data is in use by a running Chrome",
            )
            .param("name", name.clone()));
        }
    }
    if generated(app) {
        if let Ok(resolved) = resolve(app) {
            if resolved.profile_dir.starts_with(&root) {
                check_singleton(&resolved.profile_dir)?;
            }
        }
    }
    Ok(ResourceLock { _guard: guard })
}

#[derive(Clone, Debug, Serialize)]
pub struct Installation {
    pub status: &'static str,
    pub binary: Option<PathBuf>,
}
pub fn installation(env: &BTreeMap<OsString, OsString>) -> Installation {
    let mut non_executable = None;
    let Some(path) = env.get(std::ffi::OsStr::new("PATH")) else {
        return Installation {
            status: "unknown",
            binary: None,
        };
    };
    for dir in std::env::split_paths(path).filter(|dir| dir.is_absolute()) {
        let binary = dir.join("google-chrome");
        if !binary.is_file() {
            continue;
        }
        if super::launch::accessible(&binary, libc::X_OK) {
            return Installation {
                status: "installed",
                binary: Some(binary),
            };
        }
        non_executable = Some(binary);
    }
    Installation {
        status: if non_executable.is_some() {
            "not_executable"
        } else {
            "missing"
        },
        binary: non_executable,
    }
}

/// Source precisely the profiles used by real app launches, without executing
/// Chrome. Bound both time and output; shell startup failures are "unknown",
/// never a false claim that the binary is missing. Results are briefly cached.
pub fn installation_for_app(app: &ManagedApp) -> Installation {
    use std::{
        hash::{Hash, Hasher},
        io::Read,
        process::Stdio,
        time::{Duration, Instant},
    };
    let home = crate::paths::app_home(&app.id);
    let env = super::native::app_environment(app, &home, std::env::vars_os());
    let real = std::env::var_os("HOME").map(PathBuf::from);
    let homes = crate::shell_profile::app_profile_homes(real.as_deref(), &home);
    if homes.is_empty() {
        return installation(&env);
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    env.hash(&mut hash);
    homes.hash(&mut hash);
    for dir in &homes {
        for name in [".profile", ".bashrc"] {
            fs::metadata(dir.join(name))
                .ok()
                .and_then(|m| m.modified().ok())
                .hash(&mut hash);
        }
    }
    let key = hash.finish();
    type Cache = HashMap<u64, (Instant, Installation)>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some((time, value)) = cache.lock().unwrap().get(&key) {
        if time.elapsed() < Duration::from_secs(3) {
            return value.clone();
        }
    }
    let result = (|| -> Option<Installation> {
        let fd = unsafe { libc::memfd_create(c"ivnc-chrome-probe".as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return None;
        }
        let output = unsafe { File::from_raw_fd(fd) };
        let mut command = crate::shell_profile::app_command(
            "builtin printf '\\nIVNC_CHROME_BINARY\\n'; builtin type -P google-chrome",
            &homes,
        );
        command
            .env_clear()
            .envs(&env)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .stdout(output.try_clone().ok()?);
        super::native::configure_app_session(&mut command);
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                let limit = libc::rlimit {
                    rlim_cur: 65536,
                    rlim_max: 65536,
                };
                if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            });
        }
        struct Probe(std::process::Child);
        impl Drop for Probe {
            fn drop(&mut self) {
                // Profile files may leave background children behind even
                // after bash exits. Reap the complete private probe group.
                unsafe {
                    libc::kill(-(self.0.id() as i32), libc::SIGKILL);
                }
                let _ = self.0.wait();
            }
        }
        let mut child = Probe(command.spawn().ok()?);
        let until = Instant::now() + Duration::from_secs(2);
        let success = loop {
            if let Some(status) = child.0.try_wait().ok()? {
                break status.success();
            }
            if Instant::now() >= until {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        drop(child);
        use std::io::{Seek, SeekFrom};
        let mut output = output;
        output.seek(SeekFrom::Start(0)).ok()?;
        let mut text = String::new();
        output.take(65536).read_to_string(&mut text).ok()?;
        let path = text.rsplit_once("\nIVNC_CHROME_BINARY\n")?.1.trim();
        if !success || path.is_empty() {
            let base = installation(&env);
            return Some(Installation {
                status: if base.status == "not_executable" {
                    "not_executable"
                } else {
                    "missing"
                },
                binary: None,
            });
        }
        let path = PathBuf::from(path);
        let path = if path.is_absolute() {
            path
        } else {
            home.join(path)
        };
        Some(Installation {
            status: if path.is_file() && super::launch::accessible(&path, libc::X_OK) {
                "installed"
            } else {
                "not_executable"
            },
            binary: Some(path),
        })
    })()
    .unwrap_or(Installation {
        status: "unknown",
        binary: None,
    });
    let mut cache = cache.lock().unwrap();
    cache.retain(|_, (time, _)| time.elapsed() < Duration::from_secs(3));
    cache.insert(key, (Instant::now(), result.clone()));
    result
}

/// Only a known legacy template may be converted into editable settings.
/// Every unrecognized shell command remains manual and is preserved verbatim.
pub fn migrate_builtin(app: &mut ManagedApp) -> Result<(), ApiError> {
    if app.chrome.is_some() {
        return refresh_command(app);
    }
    let mut config = ChromeConfig::builtin();
    let original = app.exec_command.as_deref().unwrap_or("");
    // Tokenization is only used for strict template recognition, never execution.
    let known = legacy_config(original, app.proxy_url.as_deref());
    if let Some((legacy, proxy)) = known {
        config = legacy;
        app.proxy_url = proxy;
    } else if !original.is_empty() {
        config.mode = CommandMode::Manual;
    }
    app.chrome = Some(config);
    if let Err(error) = refresh_command(app) {
        // An old custom path/proxy that the new structured validator cannot
        // safely accept must not prevent iVnc itself from starting.
        if app.exec_command.is_some() {
            app.chrome.as_mut().unwrap().mode = CommandMode::Manual;
            Ok(())
        } else {
            Err(error)
        }
    } else {
        Ok(())
    }
}

pub(super) fn legacy_config(
    command: &str,
    proxy: Option<&str>,
) -> Option<(ChromeConfig, Option<String>)> {
    let command = command.trim();
    let (profile, rest) = literal(command.strip_prefix("profile_dir=")?)?;
    let (crash, rest) = literal(rest.strip_prefix("; crash_dir=")?)?;
    let mut rest = rest.strip_prefix("; rm -f \"$profile_dir/SingletonLock\" \"$profile_dir/SingletonSocket\" \"$profile_dir/SingletonCookie\"; exec google-chrome")?;
    let parsed_proxy = if let Some(value) = rest.strip_prefix(" --proxy-server=") {
        let (value, tail) = literal(value)?;
        rest = tail;
        Some(value)
    } else {
        None
    };
    rest = rest.strip_prefix(" --user-data-dir=\"$profile_dir\" --crash-dumps-dir=\"$crash_dir\" --remote-debugging-host=0.0.0.0 --remote-debugging-port=")?;
    let (port, rest) = rest.split_once(' ')?;
    let port = port.parse::<u16>().ok().filter(|p| *p != 0)?;
    if rest != "--ozone-platform=wayland --class=ivnc-chrome-windowed --test-type --no-first-run --no-default-browser-check --disable-features=MediaRouter --disable-background-networking --disable-process-singleton --no-sandbox --disable-setuid-sandbox --disable-dev-shm-usage" {
        return None;
    }
    let proxy_arg = parsed_proxy
        .as_deref()
        .map(|p| format!(" --proxy-server={}", shell_quote(p)))
        .unwrap_or_default();
    let expected = format!("profile_dir={}; crash_dir={}; rm -f \"$profile_dir/SingletonLock\" \"$profile_dir/SingletonSocket\" \"$profile_dir/SingletonCookie\"; exec google-chrome{proxy_arg} --user-data-dir=\"$profile_dir\" --crash-dumps-dir=\"$crash_dir\" --remote-debugging-host=0.0.0.0 --remote-debugging-port={port} {rest}", shell_quote(&profile), shell_quote(&crash));
    if command != expected {
        return None;
    }
    // The old proxy column is authoritative when present; otherwise recover
    // the literal URL from the recognized template.
    Some((
        ChromeConfig {
            mode: CommandMode::Generated,
            profile_dir: Some(profile),
            crash_dir: Some(crash),
            debug_port: Some(port),
        },
        proxy.map(str::to_owned).or(parsed_proxy),
    ))
}

fn literal(input: &str) -> Option<(String, &str)> {
    let mut value = String::new();
    let mut quote = None;
    let mut chars = input.char_indices();
    while let Some((index, ch)) = chars.next() {
        match (quote, ch) {
            (None, ';' | ' ') => return Some((value, &input[index..])),
            (None, '\'' | '"') => quote = Some(ch),
            (Some(q), c) if q == c => quote = None,
            (None, '\\') => value.push(chars.next()?.1),
            (Some('"'), '$' | '`') | (None, '$' | '`' | '\n' | '\r') => return None,
            (_, c) => value.push(c),
        }
    }
    quote.is_none().then_some((value, ""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::{app::AppType, store::AppStore};
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn fixture(test: &str) -> Option<PathBuf> {
        if let Some(root) = std::env::var_os("IVNC_CHROME_TEST_CHILD") {
            return Some(root.into());
        }
        let root =
            std::env::temp_dir().join(format!("ivnc-chrome-preset-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("home")).unwrap();
        fs::create_dir_all(root.join("bin")).unwrap();
        let binary = root.join("bin/google-chrome");
        fs::write(
            &binary,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$CHROME_ARGV_OUT\"\n",
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("apps::chrome::tests::{test}"),
                "--test-threads=1",
            ])
            .env("IVNC_CHROME_TEST_CHILD", &root)
            .env("IVNC_HOME", root.join("ivnc"))
            .env("HOME", root.join("home"))
            .env("PATH", root.join("bin"))
            .env("CHROME_ARGV_OUT", root.join("argv"))
            .output()
            .unwrap();
        let _ = fs::remove_dir_all(root);
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        None
    }

    fn app(id: &str) -> ManagedApp {
        let mut app = ManagedApp::for_test(id, AppType::DesktopApp);
        app.name = id.into();
        app.chrome = Some(ChromeConfig::default());
        refresh_command(&mut app).unwrap();
        app
    }

    #[test]
    fn defaults_are_isolated_and_preview_has_no_directory_side_effects() {
        let Some(_) = fixture("defaults_are_isolated_and_preview_has_no_directory_side_effects")
        else {
            return;
        };
        let first = app("first");
        let second = app("second");
        let a = resolve(&first).unwrap();
        let b = resolve(&second).unwrap();
        assert_ne!(a.profile_dir, b.profile_dir);
        assert_ne!(a.crash_dir, b.crash_dir);
        assert!(a.profile_dir.starts_with(&a.app_root));
        assert!(!a.app_root.exists());
        assert!(!first.exec_command.unwrap().contains("--remote-debugging"));
    }

    #[test]
    fn command_quoting_and_manual_mode_are_authoritative() {
        let Some(root) = fixture("command_quoting_and_manual_mode_are_authoritative") else {
            return;
        };
        let mut first = app("first");
        first.chrome.as_mut().unwrap().profile_dir = Some(
            root.join("profile ' ; $(touch pwned)")
                .to_string_lossy()
                .into_owned(),
        );
        first.proxy_url = Some("socks5://user:p%27ass@localhost:1080".into());
        let lease = acquire(&mut first, &[]).unwrap().unwrap();
        let status = std::process::Command::new("/bin/bash")
            .args(["-c", first.exec_command.as_ref().unwrap()])
            .current_dir(&root)
            .env("HOME", root.join("home"))
            .status()
            .unwrap();
        assert!(status.success());
        assert!(!root.join("pwned").exists());
        let args = fs::read_to_string(root.join("argv")).unwrap();
        assert!(args.contains("profile ' ; $(touch pwned)"));
        assert!(args.contains("--proxy-server=socks5://user:p%27ass@localhost:1080"));
        drop(lease);
        first.proxy_url = Some("socks5h://localhost:1080".into());
        refresh_command(&mut first).unwrap();
        assert!(first
            .exec_command
            .as_ref()
            .unwrap()
            .contains("--proxy-server='socks5://localhost:1080'"));
        assert!(!first.exec_command.as_ref().unwrap().contains("socks5h://"));
        let manual = "printf 'custom command'\n# keep exact whitespace  \n";
        apply_input(&mut first, &serde_json::json!({"chrome_config":{"mode":"manual"}, "exec_command":manual, "proxy_url":"invalid inactive proxy"})).unwrap();
        refresh_command(&mut first).unwrap();
        assert_eq!(first.exec_command.as_deref(), Some(manual));
        assert!(acquire(&mut first, &[]).unwrap().is_none());
        assert_eq!(
            cleanup_lock(&first).unwrap_err().code,
            "chrome_manual_cleanup"
        );
        let plan = crate::apps::compat::plan(&first, &BTreeMap::new(), 0).unwrap();
        assert_eq!(plan.effective_command, manual);
        assert!(plan.added_args.is_empty());
    }

    #[test]
    fn saved_and_running_resources_prevent_collisions_after_edits() {
        let Some(_) = fixture("saved_and_running_resources_prevent_collisions_after_edits") else {
            return;
        };
        let store = AppStore::in_memory();
        let mut first = app("first");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        first.chrome.as_mut().unwrap().debug_port = Some(port);
        refresh_command(&mut first).unwrap();
        store.add(&first).unwrap();
        let mut second = app("second");
        second.chrome.as_mut().unwrap().debug_port = Some(port);
        assert_eq!(store.add(&second).unwrap_err().code, "chrome_port_conflict");
        let lease = acquire(&mut first.clone(), &store.list().unwrap())
            .unwrap()
            .unwrap();
        let old_profile = resolve(&first).unwrap().profile_dir;
        first.chrome.as_mut().unwrap().debug_port = None;
        first.chrome.as_mut().unwrap().profile_dir = Some(
            resolve(&first)
                .unwrap()
                .app_root
                .join("new-profile")
                .to_string_lossy()
                .into_owned(),
        );
        refresh_command(&mut first).unwrap();
        store.update(&first).unwrap();
        assert_eq!(store.add(&second).unwrap_err().code, "chrome_port_conflict");
        drop(lease);
        store.add(&second).unwrap();
        assert!(
            old_profile.is_dir(),
            "editing a profile never deletes old data"
        );
        let mut third = app("third");
        third.chrome.as_mut().unwrap().profile_dir =
            Some(old_profile.to_string_lossy().into_owned());
        assert_eq!(
            store.add(&third).unwrap_err().code,
            "chrome_directory_conflict"
        );
    }

    #[test]
    fn aliases_live_profiles_and_occupied_ports_are_rejected() {
        let Some(root) = fixture("aliases_live_profiles_and_occupied_ports_are_rejected") else {
            return;
        };
        let store = AppStore::in_memory();
        let mut first = app("first");
        fs::create_dir_all(resolve(&first).unwrap().profile_dir).unwrap();
        store.add(&first).unwrap();
        symlink(resolve(&first).unwrap().profile_dir, root.join("alias")).unwrap();
        let mut second = app("second");
        second.chrome.as_mut().unwrap().profile_dir =
            Some(root.join("alias").to_string_lossy().into_owned());
        assert_eq!(
            store.add(&second).unwrap_err().code,
            "chrome_directory_conflict"
        );
        let lock = resolve(&first).unwrap().profile_dir.join("SingletonLock");
        let host = fs::read_to_string("/proc/sys/kernel/hostname").unwrap();
        symlink(format!("{}-{}", host.trim(), std::process::id()), &lock).unwrap();
        assert_eq!(
            acquire(&mut first, &store.list().unwrap())
                .unwrap_err()
                .code,
            "chrome_profile_busy"
        );
        assert!(fs::read_link(&lock).is_ok());
        fs::remove_file(lock).unwrap();
        let listener = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
        first.chrome.as_mut().unwrap().debug_port = Some(listener.local_addr().unwrap().port());
        assert_eq!(
            acquire(&mut first, &store.list().unwrap())
                .unwrap_err()
                .code,
            "chrome_port_in_use"
        );
    }

    #[test]
    fn clearing_one_instance_preserves_others_and_host_data() {
        let Some(root) = fixture("clearing_one_instance_preserves_others_and_host_data") else {
            return;
        };
        let mut first = app("first");
        let mut second = app("second");
        let lease_a = acquire(&mut first, &[]).unwrap().unwrap();
        let lease_b = acquire(&mut second, &[]).unwrap().unwrap();
        let a = resolve(&first).unwrap();
        let b = resolve(&second).unwrap();
        fs::create_dir_all(a.profile_dir.join("Default/Cache")).unwrap();
        fs::write(a.profile_dir.join("Default/Cache/data"), b"cache").unwrap();
        fs::create_dir_all(a.profile_dir.join("Default/Extensions")).unwrap();
        fs::write(a.profile_dir.join("Default/Extensions/keep"), b"extension").unwrap();
        fs::write(b.temp_dir.join("keep"), b"other runtime").unwrap();
        let host = root.join("home/.config/google-chrome");
        fs::create_dir_all(&host).unwrap();
        fs::write(host.join("keep"), b"host").unwrap();
        let global = std::env::temp_dir().join(format!(
            "com.google.Chrome.ivnc-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&global).unwrap();
        fs::write(global.join("keep"), b"socket").unwrap();
        assert_eq!(
            crate::apps::datadir::clear(
                &first,
                &crate::apps::datadir::ClearSelection::default_chrome()
            )
            .unwrap_err()
            .code,
            "chrome_profile_busy"
        );
        drop(lease_a);
        crate::apps::datadir::clear(
            &first,
            &crate::apps::datadir::ClearSelection::default_chrome(),
        )
        .unwrap();
        assert!(!a.profile_dir.join("Default/Cache").exists());
        assert!(a.profile_dir.join("Default/Extensions/keep").exists());
        assert!(b.temp_dir.join("keep").exists());
        assert!(host.join("keep").exists());
        assert!(global.join("keep").exists());
        fs::remove_dir_all(global).unwrap();
        drop(lease_b);
    }

    #[test]
    fn external_cleanup_is_bounded_and_symlinks_cannot_escape_profile() {
        let Some(root) = fixture("external_cleanup_is_bounded_and_symlinks_cannot_escape_profile")
        else {
            return;
        };
        let mut first = app("first");
        first.chrome.as_mut().unwrap().profile_dir =
            Some(root.join("external").to_string_lossy().into_owned());
        fs::create_dir_all(root.join("external/Default")).unwrap();
        assert_eq!(
            cleanup_lock(&first).unwrap_err().code,
            "chrome_directory_unmanaged"
        );
        // Deleting only the application's root is allowed even when the
        // external profile has never been claimed or subsequently goes away.
        drop(managed_root_lock(&first).unwrap());
        fs::remove_dir_all(root.join("external")).unwrap();
        fs::write(root.join("external"), b"unavailable profile").unwrap();
        drop(managed_root_lock(&first).unwrap());
        fs::remove_file(root.join("external")).unwrap();
        fs::create_dir_all(root.join("external/Default")).unwrap();
        let lease = acquire(&mut first, &[]).unwrap().unwrap();
        assert_eq!(
            managed_root_lock(&first).unwrap_err().code,
            "chrome_profile_busy"
        );
        drop(lease);
        assert_eq!(
            crate::apps::datadir::clear(
                &first,
                &crate::apps::datadir::ClearSelection {
                    categories: vec![],
                    full_reset: true
                }
            )
            .unwrap_err()
            .code,
            "chrome_external_full_reset"
        );
        let outside = root.join("outside");
        fs::create_dir_all(outside.join("Cache")).unwrap();
        fs::write(outside.join("Cache/keep"), b"outside").unwrap();
        fs::remove_dir(root.join("external/Default")).unwrap();
        symlink(&outside, root.join("external/Default")).unwrap();
        assert!(crate::apps::datadir::clear(
            &first,
            &crate::apps::datadir::ClearSelection::default_chrome()
        )
        .is_err());
        assert!(outside.join("Cache/keep").exists());
    }

    #[test]
    fn executable_status_observes_permissions_and_launch_profiles() {
        let Some(root) = fixture("executable_status_observes_permissions_and_launch_profiles")
        else {
            return;
        };
        let env = BTreeMap::from([(OsString::from("PATH"), root.join("bin").into_os_string())]);
        assert_eq!(installation(&env).status, "installed");
        fs::set_permissions(
            root.join("bin/google-chrome"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert_eq!(installation(&env).status, "not_executable");
        fs::set_permissions(
            root.join("bin/google-chrome"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        let first = app("first");
        fs::write(
            root.join("home/.bashrc"),
            format!(
                "export PATH={}\n",
                shell_quote(&root.join("bin").to_string_lossy())
            ),
        )
        .unwrap();
        assert_eq!(installation_for_app(&first).status, "installed");
        fs::write(
            root.join("home/.bashrc"),
            "export PATH=/ivnc-missing-path\n",
        )
        .unwrap();
        assert_eq!(installation_for_app(&first).status, "missing");
        let pid_file = root.join("probe-child");
        fs::write(root.join("home/.bashrc"), format!(
            "/bin/sleep 60 >/dev/null 2>&1 &\nbuiltin printf '%s' \"$!\" > {}\nexport PATH={}\n",
            shell_quote(&pid_file.to_string_lossy()), shell_quote(&root.join("bin").to_string_lossy())
        )).unwrap();
        assert_eq!(installation_for_app(&first).status, "installed");
        let pid = fs::read_to_string(&pid_file).unwrap();
        let stopped = || {
            fs::read_to_string(format!("/proc/{pid}/stat")).map_or(true, |s| {
                s.rsplit_once(')').unwrap().1.trim_start().starts_with('Z')
            })
        };
        for _ in 0..20 {
            if stopped() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            stopped(),
            "the installation probe must stop shell-profile children"
        );
    }
}
