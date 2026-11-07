//! One compositor and one sandbox per managed desktop application.
//! The supervisor owns the host-side network helpers. Only application data,
//! read-only system files and explicitly selected service sockets enter the sandbox.
use super::store::{private_write, DesktopSettings, RemoteStore};
use crate::{
    apps::app::{AppType, ManagedApp},
    config::Config,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    net::SocketAddr,
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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Launch {
    pub app: ManagedApp,
    pub config: Config,
    pub desktop: DesktopSettings,
    pub services: Vec<Service>,
    pub proxy: Option<SocketAddr>,
    pub devtools: Option<u16>,
    pub denied_ips: Vec<std::net::Ipv4Addr>,
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

pub fn command(app: &ManagedApp) -> Result<Command, String> {
    if app.app_type != AppType::DesktopApp || !super::valid_id(&app.id) {
        return Err("Only managed desktop applications have a runtime".into());
    }
    crate::paths::prepare_app_home(&app.id)?;
    let root = runtime_dir(&app.id);
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    let desktop = RemoteStore::new()?.policy(&app.id)?.desktop;
    let mut config = PARENT_CONFIG.get().cloned().unwrap_or_default();
    let parent_port = config.http.port;
    config.http.port = 8008;
    config.http.host = "0.0.0.0".into();
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
    config.server.pidfile = PathBuf::from("/run/ivnc/worker.pid");
    // Resolve registered service endpoints on the host, never from caller-supplied URLs.
    let store = crate::apps::store::AppStore::new()?;
    let mut services = Vec::new();
    for id in &desktop.services {
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
    let out = Command::new("ip")
        .args(["-j", "address"])
        .output()
        .map_err(|e| format!("Cannot inspect host network: {e}"))?;
    if !out.status.success() {
        return Err("Cannot inspect host network for isolation".into());
    }
    {
        {
            let value: serde_json::Value = serde_json::from_slice(&out.stdout)
                .map_err(|e| format!("Cannot inspect host addresses: {e}"))?;
            for iface in value.as_array().into_iter().flatten() {
                for addr in iface["addr_info"].as_array().into_iter().flatten() {
                    if let Some(ip) = addr["local"].as_str().and_then(|s| s.parse().ok()) {
                        denied_ips.push(ip);
                    }
                }
            }
        }
    }
    // NAT/public aliases may not appear in `ip address`. Operators can specify
    // them explicitly; a configured public ICE endpoint is also protected.
    if let Some(candidate) = &PARENT_CONFIG
        .get()
        .and_then(|c| c.webrtc.public_candidate.as_ref())
    {
        use std::net::ToSocketAddrs;
        for address in candidate
            .to_socket_addrs()
            .map_err(|e| format!("Cannot resolve public endpoint for isolation: {e}"))?
        {
            if let std::net::IpAddr::V4(ip) = address.ip() {
                denied_ips.push(ip);
            }
        }
    }
    if let Ok(extra) = std::env::var("IVNC_DESKTOP_DENY_IPS") {
        for ip in extra.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            denied_ips
                .push(ip.parse().map_err(|_| {
                    "IVNC_DESKTOP_DENY_IPS requires comma-separated IPv4 addresses"
                })?);
        }
    }
    denied_ips.sort_unstable();
    denied_ips.dedup();
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
        desktop,
        services,
        proxy,
        devtools: (app.id == "builtin-chrome").then(crate::apps::api::chrome_devtools_port),
        denied_ips,
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
    use std::os::unix::fs::OpenOptionsExt;
    let logfile = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(crate::apps::native::log_path(&app.id))
        .map_err(|e| e.to_string())?;
    cmd.stderr(Stdio::from(logfile.try_clone().map_err(|e| e.to_string())?))
        .stdout(Stdio::from(logfile));
    Ok(cmd)
}

fn mount_if_exists(cmd: &mut Command, source: &str) {
    if Path::new(source).exists() {
        cmd.args(["--ro-bind", source, source]);
    }
}

pub async fn supervise(path: &Path) -> Result<(), String> {
    let launch: Launch = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    for binary in ["bwrap", "slirp4netns", "nft", "ip"] {
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
    use std::os::fd::AsRawFd;
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
    let control = launch.root.join("control");
    fs::create_dir_all(&control).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(&control).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().map_err(|e| e.to_string())?.is_file()
            || entry.file_type().map_err(|e| e.to_string())?.is_socket()
        {
            let _ = fs::remove_file(entry.path());
        }
    }
    private_write(
        &control.join("launch.json"),
        &serde_json::to_vec(&launch).unwrap(),
    )?;
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
                    if let Ok(Ok(mut upstream)) =
                        tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(address))
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
    let info_path = launch.root.join("namespace.json");
    let mut info = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .read(true)
        .open(&info_path)
        .map_err(|e| e.to_string())?;
    let info_fd = info.as_raw_fd();
    unsafe {
        libc::fcntl(info_fd, libc::F_SETFD, 0);
    }
    let mut cmd = Command::new("bwrap");
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
    .arg(info_fd.to_string());
    for p in [
        "/usr",
        "/bin",
        "/sbin",
        "/lib",
        "/lib64",
        "/opt",
        "/etc/fonts",
        "/etc/ssl",
        "/etc/ld.so.cache",
        "/etc/alternatives",
        "/etc/passwd",
        "/etc/group",
        "/etc/nsswitch.conf",
        "/etc/machine-id",
        "/etc/os-release",
    ] {
        mount_if_exists(&mut cmd, p);
    }
    let app_root = crate::paths::app_dir(&launch.app.id);
    cmd.args([
        "--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp", "--tmpfs", "/run",
    ]);
    cmd.arg("--bind").arg(&app_root).arg(&app_root);
    cmd.arg("--bind").arg(&control).arg("/run/ivnc");
    cmd.arg("--ro-bind")
        .arg(control.join("resolv.conf"))
        .arg("/etc/resolv.conf");
    cmd.arg("--ro-bind")
        .arg(control.join("hosts"))
        .arg("/etc/hosts");
    cmd.arg("--ro-bind")
        .arg(std::env::current_exe().map_err(|e| e.to_string())?)
        .arg("/ivnc-worker");
    cmd.args([
        "--clearenv",
        "--setenv",
        "PATH",
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        "--setenv",
        "LANG",
        "C.UTF-8",
        "--setenv",
        "XDG_RUNTIME_DIR",
        "/run/ivnc/display",
    ]);
    cmd.arg("--setenv")
        .arg("HOME")
        .arg(crate::paths::app_home(&launch.app.id));
    cmd.arg("--setenv")
        .arg("IVNC_HOME")
        .arg(app_root.join("worker-state"));
    cmd.arg("--chdir")
        .arg(crate::paths::app_home(&launch.app.id));
    cmd.args([
        "--",
        "/ivnc-worker",
        "--app-worker",
        "/run/ivnc/launch.json",
    ]);
    let mut sandbox = tokio::process::Command::from(cmd);
    sandbox.kill_on_drop(true);
    let mut child = sandbox
        .spawn()
        .map_err(|e| format!("Cannot start application sandbox: {e}"))?;
    use std::io::{Seek, SeekFrom};
    let mut namespace_pid = None;
    for _ in 0..100 {
        info.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut text = String::new();
        info.read_to_string(&mut text).map_err(|e| e.to_string())?;
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            namespace_pid = v["child-pid"].as_u64();
            if namespace_pid.is_some() {
                break;
            }
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Err(format!("Sandbox setup failed ({status}); user/mount/PID/network namespaces must be permitted"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    unsafe {
        libc::fcntl(info_fd, libc::F_SETFD, libc::FD_CLOEXEC);
    }
    let pid = namespace_pid.ok_or("Sandbox did not report its namespace PID")?;
    let api = socket_root.join("slirp.sock");
    let _ = fs::remove_file(&api);
    let mut network = tokio::process::Command::new("slirp4netns");
    network
        .args([
            "--configure",
            "--disable-host-loopback",
            "--enable-seccomp",
            "--api-socket",
        ])
        .arg(&api)
        .arg("--userns-path")
        .arg(format!("/proc/{pid}/ns/user"))
        .arg(pid.to_string())
        .arg("tap0")
        .kill_on_drop(true);
    let mut network = network
        .spawn()
        .map_err(|e| format!("Cannot start isolated networking: {e}"))?;
    let mut network_ready = false;
    for _ in 0..100 {
        if api.exists() {
            network_ready = true;
            break;
        }
        if let Some(status) = network.try_wait().map_err(|e| e.to_string())? {
            return Err(format!("Isolated networking exited: {status}"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    if !network_ready {
        return Err("Timed out configuring application network".into());
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    drop(listener);
    let mut socket = UnixStream::connect(&api).await.map_err(|e| e.to_string())?;
    let request=serde_json::json!({"execute":"add_hostfwd","arguments":{"proto":"tcp","host_addr":"127.0.0.1","host_port":port,"guest_addr":"10.0.2.100","guest_port":8008}}).to_string();
    socket
        .write_all(request.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    socket.shutdown().await.map_err(|e| e.to_string())?;
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), socket.read_to_end(&mut response))
        .await
        .map_err(|_| "Network setup timed out")?
        .map_err(|e| e.to_string())?;
    let response: serde_json::Value =
        serde_json::from_slice(&response).map_err(|e| e.to_string())?;
    if response.get("error").is_some() {
        return Err(format!("Cannot forward worker port: {response}"));
    }
    if let Some(devtools) = launch.devtools {
        let mut socket = UnixStream::connect(&api).await.map_err(|e| e.to_string())?;
        let request = serde_json::json!({"execute":"add_hostfwd","arguments":{"proto":"tcp","host_addr":"127.0.0.1","host_port":devtools,"guest_addr":"10.0.2.100","guest_port":8009}}).to_string();
        socket
            .write_all(request.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        socket.shutdown().await.map_err(|e| e.to_string())?;
        let mut result = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), socket.read_to_end(&mut result))
            .await
            .map_err(|_| "DevTools forwarding timed out")?
            .map_err(|e| e.to_string())?;
        let value: serde_json::Value =
            serde_json::from_slice(&result).map_err(|e| e.to_string())?;
        if value.get("error").is_some() {
            return Err(
                "Chrome local DevTools port is occupied; configure IVNC_CHROME_DEBUG_PORT".into(),
            );
        }
    }
    private_write(&control.join("network-ready"), b"ready")?;
    let http = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(1))
        .build()
        .map_err(|e| e.to_string())?;
    let mut ready = false;
    for _ in 0..120 {
        if let Ok(r) = http
            .get(format!("http://127.0.0.1:{port}/health"))
            .send()
            .await
        {
            if r.status().is_success() && control.join("application-ready").exists() {
                ready = true;
                break;
            }
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Err(format!("Application worker exited: {status}"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if !ready {
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
    let result = tokio::select! {
        r=child.wait()=>r.map(|s|s.success()).map_err(|e|e.to_string()),
        _=term.recv()=>Ok(true),
        _=tokio::signal::ctrl_c()=>Ok(true),
        _=network.wait()=>Err("Application network stopped".into()),
    };
    let _ = fs::remove_file(launch.root.join("endpoint.json"));
    let _ = child.kill().await;
    let _ = network.kill().await;
    if result? {
        Ok(())
    } else {
        Err("Application worker exited unexpectedly".into())
    }
}

/// Runs before any threads exist. Firewall installation is mandatory; failure
/// never falls back to the host network.
pub fn prepare(path: &Path) -> Result<Config, String> {
    let launch: Launch = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if !Path::new("/ivnc-worker").exists() || unsafe { libc::getpid() } <= 0 {
        return Err("Worker must run in its application sandbox".into());
    }
    for _ in 0..300 {
        if Path::new("/run/ivnc/network-ready").exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if !Path::new("/run/ivnc/network-ready").exists() {
        return Err("Isolated network was not initialized".into());
    }
    let denied = launch
        .denied_ips
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let extra = if denied.is_empty() {
        String::new()
    } else {
        format!("ip daddr {{ {denied} }} reject\n")
    };
    let rules=format!("table inet ivnc {{ chain output {{ type filter hook output priority 0; policy accept;\nct state established,related accept\noifname \"lo\" accept\nip daddr 10.0.2.3 udp dport 53 accept\nip daddr 10.0.2.3 tcp dport 53 accept\n{extra}ip daddr {{ 0.0.0.0/8,10.0.0.0/8,100.64.0.0/10,127.0.0.0/8,169.254.0.0/16,172.16.0.0/12,192.168.0.0/16,198.18.0.0/15,224.0.0.0/4,240.0.0.0/4 }} reject\nmeta nfproto ipv6 reject\n}}\n}}\n");
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
    fs::create_dir_all("/run/ivnc/display").map_err(|e| e.to_string())?;
    fs::set_permissions("/run/ivnc/display", fs::Permissions::from_mode(0o700))
        .map_err(|e| e.to_string())?;
    fs::create_dir_all(&launch.config.input.upload_dir).map_err(|e| e.to_string())?;
    std::env::set_var("IVNC_PROXY_PANEL_ENABLED", "0");
    if launch.config.audio.enabled {
        start_private_audio()?;
    }
    let config = launch.config.clone();
    WORKER
        .set(launch)
        .map_err(|_| "Worker already initialized")?;
    Ok(config)
}

fn start_private_audio() -> Result<(), String> {
    // Explicit modules avoid discovering or connecting to any host sound service.
    let config = "/run/ivnc/display/pulse.pa";
    fs::write(config, "load-module module-native-protocol-unix socket=/run/ivnc/display/pulse.sock auth-anonymous=1\nload-module module-null-sink sink_name=app rate=48000 channels=2\nset-default-sink app\nset-default-source app.monitor\n").map_err(|e|e.to_string())?;
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
            config,
        ])
        .spawn()
        .map_err(|e| format!("Private application audio requires PulseAudio: {e}"))?;
    for _ in 0..100 {
        if Path::new("/run/ivnc/display/pulse.sock").exists() {
            std::env::set_var("PULSE_SERVER", "unix:/run/ivnc/display/pulse.sock");
            std::env::set_var("PULSE_SOURCE", "app.monitor");
            return Ok(());
        }
        if pulse.try_wait().map_err(|e| e.to_string())?.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    let _ = pulse.kill();
    let _ = pulse.wait();
    Err("Private application audio did not become ready".into())
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
            let result = start_application_inner()
                .and_then(|_| private_write(Path::new("/run/ivnc/application-ready"), b"ready"));
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

fn start_application_inner() -> Result<(), String> {
    let Some(launch) = launch() else {
        return Ok(());
    };
    let mut services = launch.services.clone();
    if launch.proxy.is_some() {
        services.push(Service {
            name: "Miao proxy".into(),
            address: "127.0.0.1:1080".parse().unwrap(),
        });
    }
    let devtools = launch.devtools;
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            if let Some(port) = devtools {
                let listener = tokio::net::TcpListener::bind(("0.0.0.0", 8009)).await?;
                tokio::spawn(async move {
                    while let Ok((mut stream, _)) = listener.accept().await {
                        tokio::spawn(async move {
                            if let Ok(mut upstream) = TcpStream::connect(("127.0.0.1", port)).await
                            {
                                let _ =
                                    tokio::io::copy_bidirectional(&mut stream, &mut upstream).await;
                            }
                        });
                    }
                });
            }
            for service in services {
                let listener =
                    tokio::net::TcpListener::bind(("127.0.0.1", service.address.port())).await?;
                tokio::spawn(async move {
                    while let Ok((mut stream, _)) = listener.accept().await {
                        let port = service.address.port();
                        let proxy = service.name == "Miao proxy";
                        tokio::spawn(async move {
                            if let Ok(mut upstream) = UnixStream::connect(if proxy {
                                "/run/ivnc/miao-proxy.sock".to_string()
                            } else {
                                format!("/run/ivnc/service-{port}.sock")
                            })
                            .await
                            {
                                let _ =
                                    tokio::io::copy_bidirectional(&mut stream, &mut upstream).await;
                            }
                        });
                    }
                });
            }
            std::future::pending::<std::io::Result<()>>().await
        })
    });
    if launch.desktop.x11 {
        let mut x = Command::new("Xwayland");
        x.args([":0", "-ac", "-nolisten", "tcp", "-noreset", "-shm"]);
        let mut x = x
            .spawn()
            .map_err(|e| format!("X11 applications require Xwayland: {e}"))?;
        for _ in 0..100 {
            if Path::new("/tmp/.X11-unix/X0").exists() {
                break;
            }
            if let Some(status) = x.try_wait().map_err(|e| e.to_string())? {
                return Err(format!("Xwayland exited during startup: {status}"));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if !Path::new("/tmp/.X11-unix/X0").exists() {
            return Err("Xwayland display did not become ready".into());
        }
        Command::new("openbox")
            .env("DISPLAY", ":0")
            .arg("--sm-disable")
            .spawn()
            .map_err(|e| format!("X11 applications require openbox: {e}"))?;
    }
    let mut command = Command::new("dbus-run-session");
    if launch.proxy.is_some() {
        command.env("IVNC_CHROME_PROXY", "socks5://127.0.0.1:1080");
    }
    command.args(["--", "sh", "-c"]).arg(
        launch
            .app
            .exec_command
            .as_deref()
            .ok_or("Desktop application has no command")?,
    );
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
    if launch.desktop.x11 {
        command
            .env("DISPLAY", ":0")
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

use std::os::unix::fs::FileTypeExt;

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
