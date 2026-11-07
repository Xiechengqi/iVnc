//! One compositor and one sandbox per managed desktop application.
//! The supervisor owns the host-side network helpers. In strict isolation only
//! application data, read-only system files, explicitly allowed host paths and
//! selected service sockets enter the sandbox. Host isolation keeps the host
//! filesystem, network and environment and only privatizes the display session.
use super::store::{private_write, DesktopSettings, Isolation, Mount, RemoteStore};
use crate::{
    apps::app::{AppType, ManagedApp},
    config::Config,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    net::{IpAddr, SocketAddr},
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UnixListener, UnixStream},
};

static PARENT_CONFIG: OnceLock<Config> = OnceLock::new();
static WORKER: OnceLock<Launch> = OnceLock::new();
static READY_FD: OnceLock<RawFd> = OnceLock::new();

/// Control directory mounted read-only inside strict runtimes.
const CONTROL: &str = "/run/ivnc";
const STRICT_RUNTIME_DIR: &str = "/run/user/0";
const LOG_LIMIT: u64 = 8 * 1024 * 1024;

pub fn set_parent_config(config: Config) {
    let _ = PARENT_CONFIG.set(config);
}
pub fn is_worker() -> bool {
    WORKER.get().is_some()
}
pub fn launch() -> Option<&'static Launch> {
    WORKER.get()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Service {
    pub name: String,
    pub address: SocketAddr,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedMount {
    pub source: PathBuf,
    pub target: PathBuf,
    pub writable: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Launch {
    pub app: ManagedApp,
    pub config: Config,
    pub desktop: DesktopSettings,
    pub services: Vec<Service>,
    pub proxy: Option<SocketAddr>,
    pub devtools: Option<u16>,
    pub denied_ips: Vec<IpAddr>,
    #[serde(default)]
    pub mounts: Vec<ResolvedMount>,
    /// Environment for the worker and application, captured from the manager.
    #[serde(default)]
    pub env: Vec<(String, String)>,
    pub root: PathBuf,
    pub generation: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Endpoint {
    pub port: u16,
    pub password: String,
    pub generation: String,
    pub desktop: DesktopSettings,
    pub supervisor: u32,
}

pub fn runtime_dir(id: &str) -> PathBuf {
    crate::paths::ivnc_home().join("runtimes").join(id)
}
pub fn endpoint(id: &str) -> Result<Endpoint, String> {
    if !super::valid_id(id) {
        return Err("Invalid application ID".into());
    }
    let endpoint: Endpoint = serde_json::from_slice(
        &fs::read(runtime_dir(id).join("endpoint.json"))
            .map_err(|_| "Application runtime is not ready; check its log")?,
    )
    .map_err(|e| format!("Invalid runtime endpoint: {e}"))?;
    if unsafe { libc::kill(endpoint.supervisor as i32, 0) } != 0 {
        return Err("Application runtime has stopped".into());
    }
    Ok(endpoint)
}

pub fn service_url(app: &ManagedApp) -> Option<String> {
    if app.id == crate::proxy_panel::BUILTIN_MIAO_ID {
        let port = std::env::var("IVNC_MIAO_PORT")
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .filter(|p| *p > 0)
            .unwrap_or(crate::proxy_panel::MIAO_PORT);
        Some(format!("http://127.0.0.1:{port}/"))
    } else {
        app.url.clone()
    }
}

/// Last public IPv4 address observed by the manager. Cloud hosts often have a
/// 1:1 NAT address that is not on any interface but still reaches B's services.
fn public_ipv4_path() -> PathBuf {
    crate::paths::ivnc_home().join("public-ipv4")
}
pub fn record_public_ipv4(ip: &str) {
    if ip.parse::<std::net::Ipv4Addr>().is_ok()
        && fs::read_to_string(public_ipv4_path())
            .ok()
            .as_deref()
            .map(str::trim)
            != Some(ip)
    {
        let _ = private_write(&public_ipv4_path(), ip.as_bytes());
        log::info!("Public address {ip} is denied to new strict application runtimes; restart running applications to apply it");
    }
}

/// Host paths that must never become visible inside an application runtime.
pub fn protected_paths() -> Vec<PathBuf> {
    let mut paths = vec![
        crate::paths::ivnc_home(),
        crate::paths::host_home().join(".miao"),
    ];
    for key in [
        "IVNC_TLS_KEY_FILE",
        "IVNC_TLS_CERT_FILE",
        "IVNC_REMOTE_CA_FILE",
    ] {
        if let Some(path) = std::env::var_os(key) {
            paths.push(PathBuf::from(path));
        }
    }
    paths
        .into_iter()
        .map(|p| fs::canonicalize(&p).unwrap_or(p))
        .collect()
}

const SYSTEM_DIRS: &[&str] = &[
    "/usr", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/libx32", "/etc", "/opt", "/boot",
];
/// Kernel and runtime trees that are never shared, including service sockets in /run.
const PSEUDO_DIRS: &[&str] = &["/proc", "/sys", "/dev", "/run"];

fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// Resolves allowed host paths. The source is canonicalized so a symlink cannot
/// redirect a mount into protected data after it was approved.
pub fn resolve_mounts(
    mounts: &[Mount],
    protected: &[PathBuf],
) -> Result<Vec<ResolvedMount>, String> {
    let mut resolved = Vec::new();
    for mount in mounts {
        let target = PathBuf::from(&mount.path);
        let source = fs::canonicalize(&target)
            .map_err(|e| format!("Allowed path {} is unavailable: {e}", mount.path))?;
        for path in [&target, &source] {
            // Subdirectories of /tmp may be shared; /tmp itself stays private.
            if path == Path::new("/")
                || path == Path::new("/tmp")
                || PSEUDO_DIRS.iter().any(|d| overlaps(path, Path::new(d)))
                || path.starts_with("/ivnc-worker")
            {
                return Err(format!(
                    "{} cannot be shared with an application",
                    mount.path
                ));
            }
            if protected.iter().any(|p| overlaps(path, p)) {
                return Err(format!(
                    "{} contains iVnc or proxy credentials and cannot be shared",
                    mount.path
                ));
            }
            if mount.writable && SYSTEM_DIRS.iter().any(|d| overlaps(path, Path::new(d))) {
                return Err(format!(
                    "System path {} can only be shared read-only",
                    mount.path
                ));
            }
        }
        resolved.push(ResolvedMount {
            source,
            target,
            writable: mount.writable,
        });
    }
    Ok(resolved)
}

/// Variables a strict runtime inherits: language, time zone and input method only.
fn strict_env(key: &str) -> bool {
    matches!(
        key,
        "LANG"
            | "LANGUAGE"
            | "TZ"
            | "GTK_IM_MODULE"
            | "QT_IM_MODULE"
            | "XMODIFIERS"
            | "SDL_IM_MODULE"
            | "GLFW_IM_MODULE"
    ) || key.starts_with("LC_")
}

/// Variables never inherited, even by host runtimes: they belong to the manager's session.
fn private_env(key: &str) -> bool {
    key.starts_with("IVNC_")
        || crate::paths::APP_ENV_REMOVE.contains(&key)
        || matches!(
            key,
            "HOME"
                | "XDG_RUNTIME_DIR"
                | "WAYLAND_DISPLAY"
                | "DISPLAY"
                | "PULSE_SERVER"
                | "PULSE_SOURCE"
                | "DBUS_SESSION_BUS_ADDRESS"
                | "DBUS_SYSTEM_BUS_ADDRESS"
                | "LD_PRELOAD"
                | "GTK_CSD"
                | "PWD"
                | "OLDPWD"
                | "SHLVL"
                | "_"
        )
        || key.starts_with("TMUX")
}

fn inherited_env(isolation: Isolation) -> Vec<(String, String)> {
    let mut env: Vec<_> = std::env::vars()
        .filter(|(k, _)| !private_env(k))
        .filter(|(k, _)| isolation == Isolation::Host || strict_env(k))
        .filter(|(k, _)| k != "PATH")
        .collect();
    if !env.iter().any(|(k, _)| k == "LANG") {
        env.push(("LANG".into(), "C.UTF-8".into()));
    }
    env.sort();
    env
}

fn host_addresses() -> Result<Vec<IpAddr>, String> {
    let out = Command::new("ip")
        .args(["-j", "address"])
        .output()
        .map_err(|e| format!("Cannot inspect host network: {e}"))?;
    if !out.status.success() {
        return Err("Cannot inspect host network for isolation".into());
    }
    let value: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("Cannot inspect host addresses: {e}"))?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|iface| iface["addr_info"].as_array().into_iter().flatten())
        .filter_map(|addr| addr["local"].as_str().and_then(|s| s.parse().ok()))
        .collect())
}

pub fn command(app: &ManagedApp) -> Result<Command, String> {
    if app.app_type != AppType::DesktopApp || !super::valid_id(&app.id) {
        return Err("Only managed desktop applications have a runtime".into());
    }
    crate::paths::prepare_app_home(&app.id)?;
    let root = runtime_dir(&app.id);
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    let desktop = RemoteStore::new()?.policy(&app.id)?.desktop;
    desktop.validate()?;
    let strict = desktop.isolation == Isolation::Strict;
    let mut config = PARENT_CONFIG.get().cloned().unwrap_or_default();
    let parent_port = config.http.port;
    // Host runtimes share the host network; the supervisor assigns a loopback port.
    config.http.port = if strict { 8008 } else { 0 };
    config.http.host = if strict { "0.0.0.0" } else { "127.0.0.1" }.into();
    config.http.tls = false;
    config.http.basic_auth_enabled = true;
    config.http.basic_auth_user = "worker".into();
    config.http.basic_auth_password = super::token();
    config.input.enable_commands = false;
    config.input.upload_dir = crate::paths::app_home(&app.id)
        .join("Downloads")
        .to_string_lossy()
        .into();
    config.terminal.enabled = false;
    config.ui_features.terminal_enabled = false;
    config.ui_features.console_enabled = false;
    config.ui_features.proxy_enabled = false;
    config.webrtc.public_candidate = None;
    config.webrtc.candidate_from_host_header = false;
    config.server.pidfile = root.join("worker.pid");
    // Resolve registered service endpoints on the host, never from caller-supplied URLs.
    let store = crate::apps::store::AppStore::new()?;
    let mut services = Vec::new();
    for id in desktop.services.iter().filter(|_| strict) {
        let service = store.get(id)?;
        if service.app_type != AppType::BackgroundApp {
            return Err("Only registered background services may be allowed".into());
        }
        let url = service_url(&service).ok_or("Background service has no URL")?;
        let url = reqwest::Url::parse(&url).map_err(|_| "Invalid background service URL")?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("Service must have an HTTP(S) URL without embedded credentials".into());
        }
        use std::net::ToSocketAddrs;
        let host = url
            .host_str()
            .ok_or("Missing service host")?
            .trim_matches(['[', ']']);
        let port = url.port_or_known_default().ok_or("Missing service port")?;
        if port == parent_port || port == 8008 || port == 8009 || port == 1080 {
            return Err(
                "The iVnc management port cannot be exposed as a background service".into(),
            );
        }
        let address = (host, port)
            .to_socket_addrs()
            .map_err(|e| e.to_string())?
            .find(|a| a.ip().is_loopback())
            .ok_or("Allowed services must listen on B's loopback address")?;
        if services.iter().any(|s: &Service| s.address.port() == port) {
            continue;
        }
        services.push(Service {
            name: service.name,
            address,
        });
    }
    let mut denied_ips = Vec::new();
    let mut mounts = Vec::new();
    if strict {
        denied_ips = host_addresses()?;
        // NAT/public aliases may not appear in `ip address`. Operators can specify
        // them explicitly; the configured ICE endpoint and observed public address
        // are also protected.
        if let Some(candidate) = &PARENT_CONFIG
            .get()
            .and_then(|c| c.webrtc.public_candidate.as_ref())
        {
            use std::net::ToSocketAddrs;
            denied_ips.extend(
                candidate
                    .to_socket_addrs()
                    .map_err(|e| format!("Cannot resolve public endpoint for isolation: {e}"))?
                    .map(|a| a.ip()),
            );
        }
        if let Some(ip) = fs::read_to_string(public_ipv4_path())
            .ok()
            .and_then(|s| s.trim().parse().ok())
        {
            denied_ips.push(ip);
        }
        if let Ok(extra) = std::env::var("IVNC_DESKTOP_DENY_IPS") {
            for ip in extra.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                denied_ips.push(
                    ip.parse().map_err(|_| {
                        "IVNC_DESKTOP_DENY_IPS requires comma-separated IP addresses"
                    })?,
                );
            }
        }
        denied_ips.sort_unstable();
        denied_ips.dedup();
        mounts = resolve_mounts(&desktop.mounts, &protected_paths())?;
    }
    let proxy = if app.id == "builtin-chrome" {
        fs::read_to_string(crate::paths::miao_dir().join("chrome-proxy"))
            .ok()
            .and_then(|s| reqwest::Url::parse(s.trim()).ok())
            .filter(|u| u.scheme() == "socks5")
            .and_then(|u| {
                Some((
                    u.host_str()?
                        .trim_matches(['[', ']'])
                        .parse::<std::net::IpAddr>()
                        .ok()?,
                    u.port()?,
                ))
            })
            .filter(|(ip, _)| ip.is_loopback())
            .map(|(ip, port)| SocketAddr::new(ip, port))
    } else {
        None
    };
    let launch = Launch {
        app: app.clone(),
        config,
        env: inherited_env(desktop.isolation),
        desktop,
        services,
        proxy,
        devtools: (app.id == "builtin-chrome").then(crate::apps::api::chrome_devtools_port),
        denied_ips,
        mounts,
        root: root.clone(),
        generation: super::token(),
    };
    private_write(
        &root.join("launch.json"),
        &serde_json::to_vec(&launch).map_err(|e| e.to_string())?,
    )?;
    let mut cmd = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    cmd.arg("--app-supervisor").arg(root.join("launch.json"));
    // Never inherit the manager's credentials, clipboard, Pulse or display sockets.
    cmd.env_clear()
        .env(
            "PATH",
            "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        )
        .env("IVNC_HOME", crate::paths::ivnc_home())
        .env("HOME", crate::paths::host_home())
        .env("LANG", "C.UTF-8");
    let logfile = open_log(&crate::apps::native::log_path(&app.id))?;
    cmd.stderr(Stdio::from(logfile.try_clone().map_err(|e| e.to_string())?))
        .stdout(Stdio::from(logfile));
    Ok(cmd)
}

/// Appends to the application log, keeping one previous generation. The log is
/// inside the application's writable data, so a planted symlink is discarded.
fn open_log(path: &Path) -> Result<fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    if let Ok(meta) = fs::symlink_metadata(path) {
        if !meta.is_file() {
            let _ = fs::remove_file(path);
        } else if meta.len() > LOG_LIMIT {
            let _ = fs::rename(path, path.with_extension("log.1"));
        }
    }
    fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| format!("Cannot open application log: {e}"))
}

fn mount_if_exists(cmd: &mut Command, source: &str) {
    if Path::new(source).exists() {
        cmd.args(["--ro-bind", source, source]);
    }
}

/// Creates a close-on-exec pipe; the caller decides which end a child inherits.
fn pipe() -> Result<(OwnedFd, OwnedFd), String> {
    let mut fds = [0; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// Lets a child inherit the given descriptors across exec.
fn inherit_fds(cmd: &mut tokio::process::Command, fds: Vec<RawFd>) {
    unsafe {
        cmd.pre_exec(move || {
            for fd in &fds {
                if libc::fcntl(*fd, libc::F_SETFD, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
}

/// Reads a short message from a pipe without blocking the runtime. Writers may
/// keep the pipe open, so reading stops once `complete` accepts the data or at EOF.
fn read_pipe(fd: OwnedFd, complete: fn(&[u8]) -> bool) -> tokio::sync::oneshot::Receiver<Vec<u8>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let mut file = fs::File::from(fd);
        let mut data = Vec::new();
        let mut chunk = [0; 256];
        while data.len() < 4096 && !complete(&data) {
            match file.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => data.extend_from_slice(&chunk[..n]),
            }
        }
        let _ = tx.send(data);
    });
    rx
}
fn any_data(data: &[u8]) -> bool {
    !data.is_empty()
}
fn json_object(data: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(data).is_ok()
}

fn free_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    Ok(listener.local_addr().map_err(|e| e.to_string())?.port())
}

async fn slirp_request(
    api: &Path,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut socket = UnixStream::connect(api).await.map_err(|e| e.to_string())?;
    socket
        .write_all(request.to_string().as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    socket.shutdown().await.map_err(|e| e.to_string())?;
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), socket.read_to_end(&mut response))
        .await
        .map_err(|_| "Network setup timed out")?
        .map_err(|e| e.to_string())?;
    serde_json::from_slice(&response).map_err(|e| e.to_string())
}

/// Forwards a free host loopback port to the runtime. Another process may take a
/// port between selection and binding, so a few ports are tried.
async fn forward_port(api: &Path, guest_port: u16) -> Result<u16, String> {
    let mut last = String::new();
    for _ in 0..5 {
        let port = free_port()?;
        let response = slirp_request(api, serde_json::json!({"execute":"add_hostfwd","arguments":{"proto":"tcp","host_addr":"127.0.0.1","host_port":port,"guest_addr":"10.0.2.100","guest_port":guest_port}})).await?;
        if response.get("error").is_none() {
            return Ok(port);
        }
        last = response.to_string();
    }
    Err(format!("Cannot forward worker port: {last}"))
}

async fn parent_exited(parent: i32) {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if unsafe { libc::getppid() } != parent {
            return;
        }
    }
}

pub async fn supervise(path: &Path) -> Result<(), String> {
    let mut launch: Launch = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    // The manager may run spawns on any thread, so lifetime is tied to the parent
    // process here rather than through a thread-bound parent-death signal.
    let parent = unsafe { libc::getppid() };
    let strict = launch.desktop.isolation == Isolation::Strict;
    let required: &[&str] = if strict {
        &["bwrap", "slirp4netns", "nft", "ip"]
    } else {
        &["bwrap"]
    };
    for binary in required {
        if !Command::new("sh")
            .arg("-c")
            .arg(format!("command -v {binary}"))
            .stdout(Stdio::null())
            .status()
            .map_err(|e| e.to_string())?
            .success()
        {
            return Err(format!("Desktop isolation requires {binary}"));
        }
    }
    // Unix socket names have a 108-byte kernel limit. A descriptor alias keeps
    // sockets inside this private runtime even when IVNC_HOME is a long path.
    let runtime_root = fs::File::open(&launch.root).map_err(|e| e.to_string())?;
    let socket_root = PathBuf::from(format!(
        "/proc/{}/fd/{}",
        std::process::id(),
        runtime_root.as_raw_fd()
    ));
    let lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(launch.root.join("runtime.lock"))
        .map_err(|e| e.to_string())?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("Application runtime is already owned by another supervisor".into());
    }
    let _ = fs::remove_file(launch.root.join("endpoint.json"));
    // Earlier runtimes mounted this directory writable; never trust what is left in it.
    let control = launch.root.join("control");
    let _ = fs::remove_dir_all(&control);
    fs::create_dir(&control).map_err(|e| e.to_string())?;
    fs::set_permissions(&control, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    // Host runtimes keep a short private runtime directory: Unix socket paths are
    // limited to 108 bytes, and a tmpfs mounted over it keeps its sockets private
    // to the sandbox and removes them with it.
    let host_runtime_dir = PathBuf::from(format!("/tmp/ivnc-{}", &launch.generation[..16]));
    if !strict {
        launch.config.http.port = free_port()?;
    }
    if strict {
        private_write(&control.join("resolv.conf"), b"nameserver 10.0.2.3\n")?;
        private_write(
            &control.join("hosts"),
            b"127.0.0.1 localhost\n::1 localhost\n",
        )?;
        // One socket per approved endpoint: the application cannot ask the
        // host broker to connect to an arbitrary destination.
        for service in &launch.services {
            let sock = socket_root
                .join("control")
                .join(format!("service-{}.sock", service.address.port()));
            let listener = UnixListener::bind(sock).map_err(|e| e.to_string())?;
            let address = service.address;
            tokio::spawn(async move {
                while let Ok((mut client, _)) = listener.accept().await {
                    tokio::spawn(async move {
                        if let Ok(Ok(mut upstream)) = tokio::time::timeout(
                            Duration::from_secs(10),
                            TcpStream::connect(address),
                        )
                        .await
                        {
                            let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
                        }
                    });
                }
            });
        }
        if let Some(address) = launch.proxy {
            let listener = UnixListener::bind(socket_root.join("control/miao-proxy.sock"))
                .map_err(|e| e.to_string())?;
            let denied = launch.denied_ips.clone();
            tokio::spawn(async move {
                let limit = std::sync::Arc::new(tokio::sync::Semaphore::new(64));
                while let Ok((stream, _)) = listener.accept().await {
                    if let Ok(permit) = limit.clone().try_acquire_owned() {
                        let denied = denied.clone();
                        tokio::spawn(async move {
                            let _permit = permit;
                            super::proxy::relay(stream, address, denied).await;
                        });
                    }
                }
            });
        }
    }
    // The launch description, including the worker password, travels through a
    // pipe and is never written where the application can read it.
    let (launch_read, launch_write) = pipe()?;
    let (ready_read, ready_write) = pipe()?;
    let (info_read, info_write) = pipe()?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let app_home = crate::paths::app_home(&launch.app.id);
    let mut cmd = Command::new("bwrap");
    if strict {
        cmd.args([
            "--unshare-all",
            "--die-with-parent",
            "--uid",
            "0",
            "--gid",
            "0",
            "--cap-drop",
            "ALL",
            "--cap-add",
            "CAP_NET_ADMIN",
            "--cap-add",
            "CAP_SETPCAP",
            "--info-fd",
        ])
        .arg(info_write.as_raw_fd().to_string());
        for p in [
            "/usr",
            "/bin",
            "/sbin",
            "/lib",
            "/lib32",
            "/lib64",
            "/libx32",
            "/opt",
            "/etc/fonts",
            "/etc/ssl",
            "/etc/ca-certificates",
            "/etc/pki",
            "/etc/ld.so.cache",
            "/etc/alternatives",
            "/etc/passwd",
            "/etc/group",
            "/etc/nsswitch.conf",
            "/etc/machine-id",
            "/etc/os-release",
            "/etc/localtime",
            "/etc/timezone",
            "/etc/locale.alias",
            "/etc/default/locale",
            "/etc/mime.types",
            "/etc/xdg",
            "/etc/gtk-3.0",
            "/etc/gtk-4.0",
            "/etc/opt/chrome",
            "/etc/chromium",
            "/etc/hostname",
        ] {
            mount_if_exists(&mut cmd, p);
        }
        cmd.args([
            "--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp", "--tmpfs", "/run",
        ]);
        for mount in &launch.mounts {
            cmd.arg(if mount.writable {
                "--bind"
            } else {
                "--ro-bind"
            })
            .arg(&mount.source)
            .arg(&mount.target);
        }
        let app_root = crate::paths::app_dir(&launch.app.id);
        cmd.arg("--bind").arg(&app_root).arg(&app_root);
        cmd.arg("--ro-bind").arg(&control).arg(CONTROL);
        cmd.arg("--ro-bind")
            .arg(control.join("resolv.conf"))
            .arg("/etc/resolv.conf");
        cmd.arg("--ro-bind")
            .arg(control.join("hosts"))
            .arg("/etc/hosts");
        cmd.arg("--ro-bind").arg(&exe).arg("/ivnc-worker");
        cmd.args([
            "--clearenv",
            "--setenv",
            "XDG_RUNTIME_DIR",
            STRICT_RUNTIME_DIR,
        ]);
    } else {
        cmd.args([
            "--dev-bind",
            "/",
            "/",
            "--proc",
            "/proc",
            "--unshare-pid",
            "--die-with-parent",
        ]);
        if unsafe { libc::geteuid() } != 0 {
            cmd.arg("--unshare-user");
        }
        if launch.desktop.x11 {
            // The application's Xwayland socket must not collide with a host X server.
            cmd.args(["--tmpfs", "/tmp/.X11-unix"]);
        }
        cmd.arg("--tmpfs")
            .arg(&host_runtime_dir)
            .arg("--clearenv")
            .arg("--setenv")
            .arg("XDG_RUNTIME_DIR")
            .arg(&host_runtime_dir);
    }
    cmd.args([
        "--setenv",
        "PATH",
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        "--setenv",
        "IVNC_SANDBOX",
        if strict { "strict" } else { "host" },
    ]);
    for (key, value) in &launch.env {
        cmd.arg("--setenv").arg(key).arg(value);
    }
    cmd.arg("--setenv")
        .arg("IVNC_READY_FD")
        .arg(ready_write.as_raw_fd().to_string());
    cmd.arg("--setenv").arg("HOME").arg(&app_home);
    cmd.arg("--setenv")
        .arg("IVNC_HOME")
        .arg(crate::paths::app_dir(&launch.app.id).join("worker-state"));
    cmd.arg("--chdir").arg(&app_home);
    cmd.arg("--")
        .arg(if strict {
            Path::new("/ivnc-worker")
        } else {
            exe.as_path()
        })
        .arg("--app-worker")
        .arg(format!("/proc/self/fd/{}", launch_read.as_raw_fd()));
    let mut sandbox = tokio::process::Command::from(cmd);
    sandbox.kill_on_drop(true);
    let mut inherited = vec![launch_read.as_raw_fd(), ready_write.as_raw_fd()];
    if strict {
        inherited.push(info_write.as_raw_fd());
    }
    inherit_fds(&mut sandbox, inherited);
    let mut child = sandbox
        .spawn()
        .map_err(|e| format!("Cannot start application sandbox: {e}"))?;
    drop((launch_read, ready_write, info_write));
    let description = serde_json::to_vec(&launch).map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        let _ = fs::File::from(launch_write).write_all(&description);
    });
    let mut ready = read_pipe(ready_read, |d| d.starts_with(b"ready"));
    let mut network = None;
    let port = if strict {
        let info = read_pipe(info_read, json_object);
        let text = tokio::select! {
            text = info => text.unwrap_or_default(),
            status = child.wait() => {
                return Err(format!("Sandbox setup failed ({}); user/mount/PID/network namespaces must be permitted", status.map_err(|e| e.to_string())?));
            }
            _ = tokio::time::sleep(Duration::from_secs(10)) => Vec::new(),
        };
        let pid = serde_json::from_slice::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v["child-pid"].as_u64())
            .ok_or("Sandbox did not report its namespace PID")?;
        let api = socket_root.join("slirp.sock");
        let _ = fs::remove_file(&api);
        let (slirp_ready_read, slirp_ready_write) = pipe()?;
        // slirp4netns exits when this descriptor closes, including if the supervisor is killed.
        let (exit_read, exit_write) = pipe()?;
        let mut slirp = tokio::process::Command::new("slirp4netns");
        slirp
            .args([
                "--configure",
                "--disable-host-loopback",
                "--enable-seccomp",
                "--ready-fd",
            ])
            .arg(slirp_ready_write.as_raw_fd().to_string())
            .arg("--exit-fd")
            .arg(exit_read.as_raw_fd().to_string())
            .arg("--api-socket")
            .arg(&api)
            .arg("--userns-path")
            .arg(format!("/proc/{pid}/ns/user"))
            .arg(pid.to_string())
            .arg("tap0")
            .kill_on_drop(true);
        inherit_fds(
            &mut slirp,
            vec![slirp_ready_write.as_raw_fd(), exit_read.as_raw_fd()],
        );
        let mut slirp = slirp
            .spawn()
            .map_err(|e| format!("Cannot start isolated networking: {e}"))?;
        drop((slirp_ready_write, exit_read));
        tokio::select! {
            data = read_pipe(slirp_ready_read, any_data) => if data.unwrap_or_default().is_empty() {
                return Err("Isolated networking did not start".into());
            },
            status = slirp.wait() => return Err(format!("Isolated networking exited: {}", status.map_err(|e| e.to_string())?)),
            _ = tokio::time::sleep(Duration::from_secs(10)) => return Err("Timed out configuring application network".into()),
        }
        let port = forward_port(&api, 8008).await?;
        if let Some(devtools) = launch.devtools {
            let value = slirp_request(&api, serde_json::json!({"execute":"add_hostfwd","arguments":{"proto":"tcp","host_addr":"127.0.0.1","host_port":devtools,"guest_addr":"10.0.2.100","guest_port":8009}})).await?;
            if value.get("error").is_some() {
                return Err(
                    "Chrome local DevTools port is occupied; configure IVNC_CHROME_DEBUG_PORT"
                        .into(),
                );
            }
        }
        network = Some((slirp, exit_write));
        private_write(&control.join("network-ready"), b"ready")?;
        port
    } else {
        launch.config.http.port
    };
    let http = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(1))
        .build()
        .map_err(|e| e.to_string())?;
    let mut application_ready = false;
    let mut healthy = false;
    for _ in 0..600 {
        if !application_ready {
            match ready.try_recv() {
                Ok(data) if data.starts_with(b"ready") => application_ready = true,
                Ok(_) | Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    return Err("Application worker exited during startup".into())
                }
                Err(_) => {}
            }
        }
        if !healthy {
            healthy = http
                .get(format!("http://127.0.0.1:{port}/health"))
                .send()
                .await
                .is_ok_and(|r| r.status().is_success());
        }
        if application_ready && healthy {
            break;
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Err(format!("Application worker exited: {status}"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if !(application_ready && healthy) {
        return Err("Application worker did not become ready".into());
    }
    let endpoint = Endpoint {
        port,
        password: launch.config.http.basic_auth_password.clone(),
        generation: launch.generation,
        desktop: launch.desktop,
        supervisor: std::process::id(),
    };
    private_write(
        &launch.root.join("endpoint.json"),
        &serde_json::to_vec(&endpoint).unwrap(),
    )?;
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|e| e.to_string())?;
    let network_stopped = async {
        match network.as_mut() {
            Some((slirp, _)) => {
                let _ = slirp.wait().await;
            }
            None => std::future::pending().await,
        }
    };
    let result = tokio::select! {
        r=child.wait()=>r.map(|s|s.success()).map_err(|e|e.to_string()),
        _=term.recv()=>Ok(true),
        _=tokio::signal::ctrl_c()=>Ok(true),
        _=parent_exited(parent)=>Ok(true),
        _=network_stopped=>Err("Application network stopped".into()),
    };
    if !strict {
        // Only the empty mount point remains on the host.
        let _ = fs::remove_dir(&host_runtime_dir);
    }
    let _ = fs::remove_file(launch.root.join("endpoint.json"));
    let _ = child.kill().await;
    if let Some((mut slirp, _)) = network {
        let _ = slirp.kill().await;
    }
    if result? {
        Ok(())
    } else {
        Err("Application worker exited unexpectedly".into())
    }
}

/// nftables rules for a strict runtime. Order matters: host addresses are denied
/// before operator-approved private networks are accepted.
pub fn firewall_rules(denied: &[IpAddr], networks: &[String]) -> Result<String, String> {
    let denied: Vec<_> = denied
        .iter()
        .filter(|ip| ip.is_ipv4())
        .map(ToString::to_string)
        .collect();
    let mut allowed = Vec::new();
    for network in networks {
        let (ip, prefix) = super::store::parse_network(network)?;
        allowed.push(format!("{ip}/{prefix}"));
    }
    let mut rules = String::from("table inet ivnc { chain output { type filter hook output priority 0; policy accept;\nct state established,related accept\noifname \"lo\" accept\nip daddr 10.0.2.3 udp dport 53 accept\nip daddr 10.0.2.3 tcp dport 53 accept\nip daddr 10.0.2.0/24 reject\n");
    if !denied.is_empty() {
        rules += &format!("ip daddr {{ {} }} reject\n", denied.join(", "));
    }
    if !allowed.is_empty() {
        rules += &format!("ip daddr {{ {} }} accept\n", allowed.join(", "));
    }
    rules += "ip daddr { 0.0.0.0/8,10.0.0.0/8,100.64.0.0/10,127.0.0.0/8,169.254.0.0/16,172.16.0.0/12,192.168.0.0/16,198.18.0.0/15,224.0.0.0/4,240.0.0.0/4 } reject\nmeta nfproto ipv6 reject\n}\n}\n";
    Ok(rules)
}

/// Runs before any threads exist. Strict firewall installation is mandatory;
/// failure never falls back to the host network.
pub fn prepare(path: &Path) -> Result<Config, String> {
    let launch: Launch = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if let Some(fd) = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.parse::<RawFd>().ok())
    {
        unsafe { libc::close(fd) };
    }
    let sandbox = std::env::var("IVNC_SANDBOX").unwrap_or_default();
    let strict = launch.desktop.isolation == Isolation::Strict;
    let expected = if strict { "strict" } else { "host" };
    if sandbox != expected {
        return Err("Worker must run in its application sandbox".into());
    }
    std::env::remove_var("IVNC_SANDBOX");
    // The readiness descriptor stays with the worker and never reaches the application.
    if let Some(fd) = std::env::var("IVNC_READY_FD")
        .ok()
        .and_then(|v| v.parse::<RawFd>().ok())
    {
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        let _ = READY_FD.set(fd);
    }
    std::env::remove_var("IVNC_READY_FD");
    if strict {
        let ready = Path::new(CONTROL).join("network-ready");
        for _ in 0..300 {
            if ready.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if !ready.exists() {
            return Err("Isolated network was not initialized".into());
        }
        let rules = firewall_rules(&launch.denied_ips, &launch.desktop.networks)?;
        let mut nft = Command::new("nft")
            .args(["-f", "-"])
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        nft.stdin
            .take()
            .unwrap()
            .write_all(rules.as_bytes())
            .map_err(|e| e.to_string())?;
        if !nft.wait().map_err(|e| e.to_string())?.success() {
            return Err("Cannot enforce application network isolation".into());
        }
        // Drop the setup capability from all subsequent descendants.
        for cap in 0..=40 {
            if unsafe { libc::prctl(libc::PR_CAPBSET_DROP, cap, 0, 0, 0) } != 0
                && std::io::Error::last_os_error().raw_os_error() != Some(libc::EINVAL)
            {
                return Err("Cannot restrict application capability bounding set".into());
            }
        }
        #[repr(C)]
        struct Header {
            version: u32,
            pid: i32,
        }
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct Caps {
            effective: u32,
            permitted: u32,
            inheritable: u32,
        }
        let header = Header {
            version: 0x20080522,
            pid: 0,
        };
        let caps = [Caps {
            effective: 0,
            permitted: 0,
            inheritable: 0,
        }; 2];
        if unsafe { libc::syscall(libc::SYS_capset, &header, caps.as_ptr()) } != 0
            || unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0
        {
            return Err("Cannot drop application setup privileges".into());
        }
        // The application shares this UID; it must not inspect the worker's memory.
        if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) } != 0 {
            return Err("Cannot protect application worker memory".into());
        }
    }
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").map_err(|_| "Missing runtime directory")?;
    fs::create_dir_all(&runtime_dir).map_err(|e| e.to_string())?;
    fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700))
        .map_err(|e| e.to_string())?;
    fs::create_dir_all(&launch.config.input.upload_dir).map_err(|e| e.to_string())?;
    std::env::set_var("IVNC_PROXY_PANEL_ENABLED", "0");
    if launch.config.audio.enabled {
        start_private_audio(Path::new(&runtime_dir))?;
    }
    let config = launch.config.clone();
    WORKER
        .set(launch)
        .map_err(|_| "Worker already initialized")?;
    Ok(config)
}

fn wait_for(path: &Path, child: &mut std::process::Child, what: &str) -> Result<(), String> {
    for _ in 0..100 {
        if path.exists() {
            return Ok(());
        }
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    let _ = child.kill();
    let _ = child.wait();
    Err(format!("{what} did not become ready"))
}

fn start_private_audio(runtime_dir: &Path) -> Result<(), String> {
    // Explicit modules avoid discovering or connecting to any host sound service.
    let config = runtime_dir.join("pulse.pa");
    let socket = runtime_dir.join("pulse.sock");
    fs::write(&config, format!("load-module module-native-protocol-unix socket={} auth-anonymous=1\nload-module module-null-sink sink_name=app rate=48000 channels=2\nset-default-sink app\nset-default-source app.monitor\n", socket.display())).map_err(|e|e.to_string())?;
    let mut pulse = Command::new("pulseaudio")
        .args([
            "--daemonize=no",
            "--use-pid-file=no",
            "--exit-idle-time=-1",
            "--disallow-exit",
            "--disable-shm",
            "--log-target=stderr",
            "-n",
            "-F",
        ])
        .arg(&config)
        .spawn()
        .map_err(|e| format!("Private application audio requires PulseAudio: {e}"))?;
    wait_for(&socket, &mut pulse, "Private application audio")?;
    std::env::set_var("PULSE_SERVER", format!("unix:{}", socket.display()));
    std::env::set_var("PULSE_SOURCE", "app.monitor");
    Ok(())
}

/// Start the application only after its private display exists.
pub fn start_application() -> Result<(), String> {
    if !is_worker() {
        return Ok(());
    }
    // Xwayland needs the compositor loop to dispatch its startup handshake.
    // Waiting for its display on the compositor thread would deadlock startup.
    std::thread::Builder::new()
        .name("app-launch".into())
        .spawn(|| {
            let result = start_application_inner().and_then(|_| {
                let fd = READY_FD.get().ok_or("Missing readiness channel")?;
                let mut ready = unsafe { fs::File::from_raw_fd(*fd) };
                ready.write_all(b"ready").map_err(|e| e.to_string())
            });
            if let Err(error) = result {
                eprintln!("Application startup failed: {error}");
                unsafe {
                    libc::kill(libc::getpid(), libc::SIGTERM);
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Starts Xwayland on the first free display and returns its number.
fn start_xwayland() -> Result<String, String> {
    use std::os::unix::process::CommandExt;
    let (read, write) = pipe()?;
    let fd = write.as_raw_fd();
    let mut x = Command::new("Xwayland");
    // Without an explicit display Xwayland chooses a free one, so a host X
    // server sharing /tmp in host isolation cannot collide with it.
    x.args(["-ac", "-nolisten", "tcp", "-noreset", "-shm", "-displayfd"])
        .arg(fd.to_string());
    unsafe {
        x.pre_exec(move || {
            if libc::fcntl(fd, libc::F_SETFD, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut x = x
        .spawn()
        .map_err(|e| format!("X11 applications require Xwayland: {e}"))?;
    drop(write);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::BufRead::read_line(
            &mut std::io::BufReader::new(fs::File::from(read)),
            &mut line,
        );
        let _ = tx.send(line);
    });
    let display = rx
        .recv_timeout(Duration::from_secs(10))
        .ok()
        .map(|line| line.trim().to_owned())
        .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    let Some(display) = display else {
        let _ = x.kill();
        let _ = x.wait();
        return Err("Xwayland display did not become ready".into());
    };
    Ok(format!(":{display}"))
}

fn start_application_inner() -> Result<(), String> {
    let Some(launch) = launch() else {
        return Ok(());
    };
    let strict = launch.desktop.isolation == Isolation::Strict;
    let mut services = launch.services.clone();
    if strict && launch.proxy.is_some() {
        services.push(Service {
            name: "Miao proxy".into(),
            address: "127.0.0.1:1080".parse().unwrap(),
        });
    }
    let devtools = launch.devtools.filter(|_| strict);
    if strict {
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                if let Some(port) = devtools {
                    let listener = tokio::net::TcpListener::bind(("0.0.0.0", 8009)).await?;
                    tokio::spawn(async move {
                        while let Ok((mut stream, _)) = listener.accept().await {
                            tokio::spawn(async move {
                                if let Ok(mut upstream) =
                                    TcpStream::connect(("127.0.0.1", port)).await
                                {
                                    let _ =
                                        tokio::io::copy_bidirectional(&mut stream, &mut upstream)
                                            .await;
                                }
                            });
                        }
                    });
                }
                for service in services {
                    let listener =
                        tokio::net::TcpListener::bind(("127.0.0.1", service.address.port()))
                            .await?;
                    tokio::spawn(async move {
                        while let Ok((mut stream, _)) = listener.accept().await {
                            let port = service.address.port();
                            let proxy = service.name == "Miao proxy";
                            tokio::spawn(async move {
                                let socket = if proxy {
                                    format!("{CONTROL}/miao-proxy.sock")
                                } else {
                                    format!("{CONTROL}/service-{port}.sock")
                                };
                                if let Ok(mut upstream) = UnixStream::connect(socket).await {
                                    let _ =
                                        tokio::io::copy_bidirectional(&mut stream, &mut upstream)
                                            .await;
                                }
                            });
                        }
                    });
                }
                std::future::pending::<std::io::Result<()>>().await
            })
        });
    }
    let runtime_dir = PathBuf::from(std::env::var("XDG_RUNTIME_DIR").unwrap_or_default());
    // A session bus independent of the application process, so launchers that
    // fork and exit do not take the bus down with them.
    let bus = runtime_dir.join("bus");
    let mut dbus = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--nopidfile"])
        .arg(format!("--address=unix:path={}", bus.display()))
        .spawn()
        .map_err(|e| format!("Application session bus requires dbus-daemon: {e}"))?;
    wait_for(&bus, &mut dbus, "Application session bus")?;
    let mut display = None;
    if launch.desktop.x11 {
        let x11 = start_xwayland()?;
        Command::new("openbox")
            .env("DISPLAY", &x11)
            .arg("--sm-disable")
            .spawn()
            .map_err(|e| format!("X11 applications require openbox: {e}"))?;
        display = Some(x11);
    }
    // The reaper owns the application's process tree: a launcher that forks and
    // exits leaves its processes with the reaper, and the application ends only
    // when the last of them exits. The worker's own helpers are not counted.
    let mut command = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    command
        .args(["--app-reaper", "sh", "-c"])
        .arg(
            launch
                .app
                .exec_command
                .as_deref()
                .ok_or("Desktop application has no command")?,
        )
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={}", bus.display()),
        )
        .env_remove("IVNC_HOME")
        .env_remove("IVNC_PROXY_PANEL_ENABLED");
    // In a strict runtime DevTools is reached through the private network's
    // forwarded loopback port; in host mode it must not listen on host interfaces.
    if !strict {
        command.env("IVNC_CHROME_DEBUG_HOST", "127.0.0.1");
    }
    if let Some(proxy) = launch.proxy {
        command.env(
            "IVNC_CHROME_PROXY",
            if strict {
                "socks5://127.0.0.1:1080".to_string()
            } else {
                format!("socks5://{proxy}")
            },
        );
    }
    if let Some(env) = &launch.app.env_vars {
        for (key, value) in env {
            if !key.starts_with("IVNC_")
                && !matches!(
                    key.as_str(),
                    "HOME"
                        | "XDG_RUNTIME_DIR"
                        | "WAYLAND_DISPLAY"
                        | "DISPLAY"
                        | "PULSE_SERVER"
                        | "DBUS_SESSION_BUS_ADDRESS"
                        | "DBUS_SYSTEM_BUS_ADDRESS"
                        | "LD_PRELOAD"
                )
            {
                command.env(key, value);
            }
        }
    }
    if let Some(display) = display {
        command
            .env("DISPLAY", display)
            .env("GDK_BACKEND", "x11")
            .env("QT_QPA_PLATFORM", "xcb")
            .env_remove("WAYLAND_DISPLAY");
    } else {
        command
            .env("GDK_BACKEND", "wayland")
            .env("QT_QPA_PLATFORM", "wayland");
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Cannot launch application: {e}"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
        unsafe {
            libc::kill(libc::getpid(), libc::SIGTERM);
        }
    });
    Ok(())
}

/// Runs an application command as a child subreaper and returns once the
/// command and every process it left behind have exited.
pub fn reap(command: Vec<std::ffi::OsString>) -> i32 {
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } != 0 {
        eprintln!("Cannot track application processes");
        return 1;
    }
    let Some((program, args)) = command.split_first() else {
        return 2;
    };
    if let Err(e) = Command::new(program).args(args).spawn() {
        eprintln!("Cannot launch application: {e}");
        return 1;
    }
    loop {
        let mut status = 0;
        if unsafe { libc::waitpid(-1, &mut status, 0) } < 0
            && std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR)
        {
            return 0;
        }
    }
}

#[cfg(test)]
mod isolation_tests {
    use super::*;

    #[test]
    fn firewall_denies_host_addresses_before_allowed_networks() {
        let rules = firewall_rules(
            &["192.168.1.5".parse().unwrap(), "fd00::1".parse().unwrap()],
            &["192.168.1.7/24".into()],
        )
        .unwrap();
        let deny = rules.find("ip daddr { 192.168.1.5 } reject").unwrap();
        let allow = rules.find("ip daddr { 192.168.1.0/24 } accept").unwrap();
        let private = rules.find("192.168.0.0/16,").unwrap();
        assert!(rules.find("10.0.2.0/24 reject").unwrap() < allow);
        assert!(deny < allow && allow < private);
        assert!(!rules.contains("fd00::1"));
        assert!(firewall_rules(&[], &["127.0.0.0/8".into()]).is_err());
    }

    #[test]
    fn mounts_reject_credentials_and_writable_system_paths() {
        let root = std::env::temp_dir().join(format!("ivnc-mount-{}", super::super::token()));
        fs::create_dir_all(root.join("data/project")).unwrap();
        fs::create_dir_all(root.join("ivnc")).unwrap();
        std::os::unix::fs::symlink(root.join("ivnc"), root.join("data/alias")).unwrap();
        let protected = vec![fs::canonicalize(root.join("ivnc")).unwrap()];
        let mount = |path: PathBuf, writable| Mount {
            path: path.to_string_lossy().into(),
            writable,
        };
        let ok = resolve_mounts(&[mount(root.join("data/project"), true)], &protected).unwrap();
        assert!(ok[0].writable);
        // A symlink cannot redirect an approved path into protected data.
        assert!(resolve_mounts(&[mount(root.join("data/alias"), false)], &protected).is_err());
        // Neither can a parent of protected data.
        assert!(resolve_mounts(&[mount(root.clone(), false)], &protected).is_err());
        assert!(resolve_mounts(&[mount("/usr/share".into(), true)], &protected).is_err());
        assert!(resolve_mounts(&[mount("/usr/share".into(), false)], &protected).is_ok());
        assert!(resolve_mounts(&[mount("/proc".into(), false)], &protected).is_err());
        assert!(resolve_mounts(&[mount("/run".into(), false)], &protected).is_err());
        assert!(resolve_mounts(&[mount("/tmp".into(), false)], &protected).is_err());
        assert!(resolve_mounts(&[mount(root.join("missing"), false)], &protected).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn strict_environment_keeps_language_and_drops_session_secrets() {
        assert!(strict_env("LANG") && strict_env("LC_TIME") && strict_env("TZ"));
        assert!(!strict_env("http_proxy") && !strict_env("AWS_SECRET_ACCESS_KEY"));
        assert!(private_env("IVNC_HOME") && private_env("WAYLAND_DISPLAY"));
        assert!(private_env("TMUX_PANE") && !private_env("http_proxy"));
    }

    #[test]
    fn logs_rotate_and_discard_planted_links() {
        let root = std::env::temp_dir().join(format!("ivnc-log-{}", super::super::token()));
        fs::create_dir_all(&root).unwrap();
        let log = root.join("app.log");
        std::os::unix::fs::symlink("/etc/passwd", &log).unwrap();
        open_log(&log).unwrap().write_all(b"one").unwrap();
        assert!(!fs::symlink_metadata(&log).unwrap().file_type().is_symlink());
        fs::write(&log, vec![0; LOG_LIMIT as usize + 1]).unwrap();
        open_log(&log).unwrap().write_all(b"two").unwrap();
        assert_eq!(fs::read(&log).unwrap(), b"two");
        assert!(root.join("app.log.1").exists());
        fs::remove_dir_all(root).unwrap();
    }
}

pub async fn files() -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(launch) = launch() else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    if !launch
        .config
        .input
        .file_transfers
        .iter()
        .any(|v| v == "download")
    {
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }
    let root = Path::new(&launch.config.input.upload_dir);
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.take(2000).flatten() {
            if entry.file_type().is_ok_and(|t| t.is_file()) {
                files.push(serde_json::json!({"name":entry.file_name().to_string_lossy(),"size":entry.metadata().map(|m|m.len()).unwrap_or(0)}));
            }
        }
    }
    axum::Json(serde_json::json!({"files":files})).into_response()
}

fn open_download(root: &Path, path: &str) -> std::io::Result<fs::File> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let root = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
        .open(root)?;
    let path = CString::new(path)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid path"))?;
    #[repr(C)]
    struct OpenHow {
        flags: u64,
        mode: u64,
        resolve: u64,
    }
    let how = OpenHow {
        flags: (libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NONBLOCK) as u64,
        mode: 0,
        resolve: 0x08 | 0x04,
    }; // BENEATH | NO_SYMLINKS
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            root.as_raw_fd(),
            path.as_ptr(),
            &how,
            std::mem::size_of::<OpenHow>(),
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let file = unsafe { fs::File::from_raw_fd(fd as i32) };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Only regular files may be downloaded",
        ));
    }
    Ok(file)
}
pub async fn download(
    axum::extract::Path(path): axum::extract::Path<String>,
) -> axum::response::Response {
    use axum::{
        body::Body,
        http::{header, StatusCode},
        response::IntoResponse,
    };
    let Some(launch) = launch() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !launch
        .config
        .input
        .file_transfers
        .iter()
        .any(|v| v == "download")
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let file = match open_download(Path::new(&launch.config.input.upload_dir), &path) {
        Ok(f) => f,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let stream =
        futures::stream::try_unfold(tokio::fs::File::from_std(file), |mut file| async move {
            let mut data = vec![0u8; 65536];
            let n = file.read(&mut data).await?;
            if n == 0 {
                Ok::<_, std::io::Error>(None)
            } else {
                data.truncate(n);
                Ok(Some((data, file)))
            }
        });
    let name = Path::new(&path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let name: String = name
        .as_bytes()
        .iter()
        .map(|b| format!("%{b:02X}"))
        .collect();
    axum::response::Response::builder()
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename*=UTF-8''{name}"),
        )
        .header(header::CONTENT_LENGTH, size)
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(stream))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn downloads_cannot_escape_through_paths_or_symlinks() {
        let root = std::env::temp_dir().join(format!("ivnc-file-test-{}", super::super::token()));
        fs::create_dir_all(root.join("downloads")).unwrap();
        fs::write(root.join("secret"), "secret").unwrap();
        fs::write(root.join("downloads/ok"), "ok").unwrap();
        std::os::unix::fs::symlink(root.join("secret"), root.join("downloads/link")).unwrap();
        std::fs::hard_link(root.join("secret"), root.join("downloads/hardlink")).unwrap();
        std::os::unix::fs::symlink(root.join("downloads"), root.join("alias")).unwrap();
        assert!(open_download(&root.join("alias"), "ok").is_err());
        assert!(open_download(&root.join("downloads"), "ok").is_ok());
        for path in ["../secret", "link", "hardlink", "/etc/passwd", "."] {
            assert!(open_download(&root.join("downloads"), path).is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }
}
