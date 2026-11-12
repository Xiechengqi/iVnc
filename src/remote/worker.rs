//! One compositor per shared desktop application. The application keeps the
//! host's filesystem, network and environment, exactly like an application of
//! the main desktop; only its display and audio session are private, so that
//! the application can be viewed on its own.
use super::store::{private_write, truncate};
use crate::{
    apps::app::{AppType, ManagedApp},
    config::Config,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
    time::{Duration, Instant},
};

static PARENT_CONFIG: OnceLock<Config> = OnceLock::new();
static WORKER: OnceLock<Launch> = OnceLock::new();
static READY_FD: OnceLock<RawFd> = OnceLock::new();

const LOG_LIMIT: u64 = 8 * 1024 * 1024;
/// Why the last start of a runtime failed before it became ready.
const FAILURE_FILE: &str = "failure.json";
const FAILURE_LENGTH: usize = 400;

/// Why a runtime failed before it became ready, for the manager to report.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
}

pub fn failure(id: &str) -> Option<Failure> {
    if !super::valid_id(id) {
        return None;
    }
    serde_json::from_slice(&fs::read(runtime_dir(id).join(FAILURE_FILE)).ok()?).ok()
}

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
pub struct Launch {
    pub app: ManagedApp,
    pub config: Config,
    pub root: PathBuf,
    pub generation: String,
    /// The application's environment apart from its session, as the manager
    /// builds it for the main desktop too; see `native::app_environment`.
    pub env: Vec<(std::ffi::OsString, std::ffi::OsString)>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Endpoint {
    pub port: u16,
    pub password: String,
    pub generation: String,
    pub supervisor: u32,
}

/// Where runtimes keep their state, one directory per application.
pub fn runtimes_dir() -> PathBuf {
    crate::paths::ivnc_home().join("runtimes")
}
pub fn runtime_dir(id: &str) -> PathBuf {
    runtimes_dir().join(id)
}
pub fn endpoint(id: &str) -> Result<Endpoint, String> {
    endpoint_in(&runtimes_dir(), id)
}
/// The endpoint a ready runtime published below `runtimes`.
pub fn endpoint_in(runtimes: &Path, id: &str) -> Result<Endpoint, String> {
    if !super::valid_id(id) {
        return Err("Invalid application ID".into());
    }
    let endpoint: Endpoint = serde_json::from_slice(
        &fs::read(runtimes.join(id).join("endpoint.json"))
            .map_err(|_| "Application runtime is not ready; check its log")?,
    )
    .map_err(|e| format!("Invalid runtime endpoint: {e}"))?;
    if unsafe { libc::kill(endpoint.supervisor as i32, 0) } != 0 {
        return Err("Application runtime has stopped".into());
    }
    Ok(endpoint)
}

/// Variables of the manager's own display, audio and bus session, and its own
/// settings. A runtime brings its own session; `IVNC_HOME` is set explicitly.
fn session_env(key: &str) -> bool {
    matches!(key, "WAYLAND_DISPLAY" | "DISPLAY" | "XDG_RUNTIME_DIR")
        || key.starts_with("PULSE_")
        || key.starts_with("DBUS_")
        || key.starts_with("IVNC_")
}

pub fn command(app: &ManagedApp) -> Result<Command, String> {
    if app.app_type != AppType::DesktopApp || !super::valid_id(&app.id) {
        return Err("Only managed desktop applications have a runtime".into());
    }
    let home = crate::paths::prepare_app_home(&app.id)?;
    let root = runtime_dir(&app.id);
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(root.join(FAILURE_FILE));
    let mut config = PARENT_CONFIG.get().cloned().unwrap_or_default();
    // The supervisor assigns a free loopback port; only it and the gateway connect.
    config.http.port = 0;
    config.http.host = "127.0.0.1".into();
    config.http.tls = false;
    config.http.basic_auth_enabled = true;
    config.http.basic_auth_user = "worker".into();
    config.http.basic_auth_password = super::token();
    config.input.enable_commands = false;
    config.input.upload_dir = home.join("Downloads").to_string_lossy().into();
    config.terminal.enabled = false;
    config.ui_features.terminal_enabled = false;
    config.ui_features.console_enabled = false;
    config.ui_features.proxy_enabled = false;
    config.webrtc.public_candidate = None;
    config.webrtc.candidate_from_host_header = false;
    config.server.pidfile = root.join("worker.pid");
    let launch = Launch {
        app: app.clone(),
        config,
        root: root.clone(),
        generation: super::token(),
        env: crate::apps::native::app_environment(app, &home, std::env::vars_os())
            .into_iter()
            .collect(),
    };
    private_write(
        &root.join("launch.json"),
        &serde_json::to_vec(&launch).map_err(|e| e.to_string())?,
    )?;
    let mut cmd = Command::new("/proc/self/exe");
    cmd.arg("--app-supervisor").arg(root.join("launch.json"));
    for (key, _) in std::env::vars_os() {
        if key.to_str().is_some_and(session_env) {
            cmd.env_remove(key);
        }
    }
    // The same HOME as in the main desktop, so both share the application's data.
    cmd.env("IVNC_HOME", crate::paths::ivnc_home())
        .env("HOME", &home);
    let logfile = open_log(&crate::apps::native::log_path(&app.id))?;
    cmd.stderr(Stdio::from(logfile.try_clone().map_err(|e| e.to_string())?))
        .stdout(Stdio::from(logfile));
    Ok(cmd)
}

/// Appends to the application log, keeping one previous generation. The log is
/// inside the application's writable data, so a planted symlink is discarded.
pub fn open_log(path: &Path) -> Result<fs::File, String> {
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

fn free_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    Ok(listener.local_addr().map_err(|e| e.to_string())?.port())
}

async fn parent_exited(parent: i32) {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if unsafe { libc::getppid() } != parent {
            return;
        }
    }
}

/// Reads the launch description and removes it: it holds the worker password.
fn read_launch(path: &Path) -> Result<Launch, String> {
    let data = fs::read(path).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(path);
    serde_json::from_slice(&data).map_err(|e| e.to_string())
}

pub async fn supervise(path: &Path) -> Result<(), String> {
    // The manager may run spawns on any thread, so lifetime is tied to the parent
    // process here rather than through a thread-bound parent-death signal.
    let parent = unsafe { libc::getppid() };
    let launch = read_launch(path)?;
    // Handled from here on, so that the supervisor outlives its own process group.
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|e| e.to_string())?;
    // The runtime is the supervisor's process group, which the manager is never
    // part of; the manager normally makes the supervisor its leader already.
    unsafe { libc::setpgid(0, 0) };
    // Processes the application leaves behind become the supervisor's children.
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } != 0 {
        return Err("Cannot track application processes".into());
    }
    let lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(launch.root.join("runtime.lock"))
        .map_err(|e| e.to_string())?;
    let root = launch.root.clone();
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(record_failure(
            &root,
            "Application runtime is already owned by another supervisor",
        ));
    }
    // Unix socket paths are limited to 108 bytes, so the display and audio
    // sockets live in a short private directory.
    let runtime_dir = PathBuf::from(format!("/tmp/ivnc-{}", &launch.generation[..16]));
    let started = tokio::select! {
        started = start_runtime(launch, &runtime_dir) => Ok(started),
        _ = term.recv() => Err(Stop::Signalled),
        _ = parent_exited(parent) => Err(Stop::Ended(Ok(()))),
    };
    let stop = match started {
        Ok(Ok(runtime)) => runtime.run(parent, &mut term).await,
        Ok(Err(message)) => Stop::Ended(Err(record_failure(&root, &message))),
        Err(stop) => stop,
    };
    let _ = fs::remove_file(root.join("endpoint.json"));
    let ended = end_process_group(matches!(stop, Stop::Signalled)).await;
    let _ = fs::remove_dir_all(&runtime_dir);
    drop(lock);
    if !ended {
        eprintln!("Application processes did not exit after SIGTERM; killing them");
        // The supervisor belongs to the group and has nothing left to do.
        unsafe { libc::kill(0, libc::SIGKILL) };
    }
    match stop {
        Stop::Signalled => Ok(()),
        Stop::Ended(result) => result,
    }
}

/// Why a supervisor stops its runtime.
enum Stop {
    /// SIGTERM, which the manager sends to the whole process group.
    Signalled,
    Ended(Result<(), String>),
}

/// Records why a runtime failed before it became ready and returns the message.
fn record_failure(root: &Path, message: &str) -> String {
    let failure = Failure {
        message: truncate(message, FAILURE_LENGTH),
    };
    let _ = private_write(
        &root.join(FAILURE_FILE),
        &serde_json::to_vec(&failure).unwrap(),
    );
    failure.message
}

/// Asks the worker, its helpers and the application to exit. The supervisor
/// leads their process group and handles SIGTERM itself. After the manager's
/// SIGTERM they first get time to exit on their own, because a second SIGTERM
/// makes Chrome abandon its orderly shutdown. Returns whether all of them
/// exited in time, well before the manager's own SIGKILL.
async fn end_process_group(signalled: bool) -> bool {
    if signalled && all_exited(Duration::from_secs(2)).await {
        return true;
    }
    unsafe { libc::kill(0, libc::SIGTERM) };
    all_exited(Duration::from_secs(if signalled { 1 } else { 3 })).await
}

async fn all_exited(within: Duration) -> bool {
    let deadline = Instant::now() + within;
    loop {
        // As a subreaper, no children left means no descendants left.
        match unsafe { libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) } {
            -1 if std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD) => {
                return true
            }
            0 | -1 if Instant::now() >= deadline => return false,
            0 | -1 => tokio::time::sleep(Duration::from_millis(50)).await,
            _ => {}
        }
    }
}

/// A runtime that became ready, and what the supervisor keeps for it.
struct Runtime {
    root: PathBuf,
    child: tokio::process::Child,
}

async fn start_runtime(mut launch: Launch, runtime_dir: &Path) -> Result<Runtime, String> {
    let _ = fs::remove_file(launch.root.join("endpoint.json"));
    // Creating it fails if anything else claimed the name first.
    fs::DirBuilder::new()
        .mode(0o700)
        .create(runtime_dir)
        .map_err(|e| format!("Cannot create application runtime directory: {e}"))?;
    launch.config.http.port = free_port()?;
    // The launch description, including the worker password, travels through a
    // pipe and is never written where the application can read it.
    let (launch_read, launch_write) = pipe()?;
    let (ready_read, ready_write) = pipe()?;
    let mut worker = tokio::process::Command::new("/proc/self/exe");
    worker
        .arg("--app-worker")
        .arg(format!("/proc/self/fd/{}", launch_read.as_raw_fd()))
        .env("XDG_RUNTIME_DIR", runtime_dir)
        .env("IVNC_READY_FD", ready_write.as_raw_fd().to_string())
        .env(
            "IVNC_HOME",
            crate::paths::app_dir(&launch.app.id).join("worker-state"),
        )
        .current_dir(crate::paths::app_home(&launch.app.id));
    inherit_fds(
        &mut worker,
        vec![launch_read.as_raw_fd(), ready_write.as_raw_fd()],
    );
    let mut child = worker
        .spawn()
        .map_err(|e| format!("Cannot start application worker: {e}"))?;
    drop((launch_read, ready_write));
    let description = serde_json::to_vec(&launch).map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        let _ = fs::File::from(launch_write).write_all(&description);
    });
    let mut ready = read_pipe(ready_read, |d| d.starts_with(b"ready"));
    let port = launch.config.http.port;
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
                Ok(data) => return Err(worker_failure(&data)),
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    return Err(worker_failure(&[]))
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
            // The worker may have reported why just before it exited.
            if !application_ready {
                if let Ok(Ok(data)) = tokio::time::timeout(Duration::from_secs(1), &mut ready).await
                {
                    if !data.starts_with(b"ready") && !data.is_empty() {
                        return Err(worker_failure(&data));
                    }
                }
            }
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
        supervisor: std::process::id(),
    };
    private_write(
        &launch.root.join("endpoint.json"),
        &serde_json::to_vec(&endpoint).unwrap(),
    )?;
    Ok(Runtime {
        root: launch.root,
        child,
    })
}

/// Why the worker failed to start the application, from what it wrote to its
/// readiness channel. The application cannot reach that channel.
fn worker_failure(data: &[u8]) -> String {
    match data.strip_prefix(STARTUP_FAILED) {
        Some(reason) => truncate(&String::from_utf8_lossy(reason), FAILURE_LENGTH),
        None => "Application worker exited during startup; see the application log".into(),
    }
}
const STARTUP_FAILED: &[u8] = b"failed: ";

impl Runtime {
    async fn run(mut self, parent: i32, term: &mut tokio::signal::unix::Signal) -> Stop {
        let stop = tokio::select! {
            r = self.child.wait() => Stop::Ended(match r {
                Ok(status) if status.success() => Ok(()),
                Ok(_) => Err("Application worker exited unexpectedly".into()),
                Err(e) => Err(e.to_string()),
            }),
            _ = term.recv() => Stop::Signalled,
            _ = tokio::signal::ctrl_c() => Stop::Ended(Ok(())),
            _ = parent_exited(parent) => Stop::Ended(Ok(())),
        };
        let _ = fs::remove_file(self.root.join("endpoint.json"));
        stop
    }
}

/// Runs before any threads exist.
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
    // The readiness descriptor stays with the worker and never reaches the application.
    if let Some(fd) = std::env::var("IVNC_READY_FD")
        .ok()
        .and_then(|v| v.parse::<RawFd>().ok())
    {
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        let _ = READY_FD.set(fd);
    }
    std::env::remove_var("IVNC_READY_FD");
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
    // Launched off the compositor thread, which starts its loop meanwhile.
    std::thread::Builder::new()
        .name("app-launch".into())
        .spawn(|| {
            let result = start_application_inner();
            // The supervisor learns the outcome, and why startup failed, over a
            // channel the application cannot reach.
            let report = match &result {
                Ok(()) => b"ready".to_vec(),
                Err(error) => [STARTUP_FAILED, error.as_bytes()].concat(),
            };
            let reported = READY_FD
                .get()
                .ok_or_else(|| "Missing readiness channel".to_string())
                .and_then(|fd| {
                    unsafe { fs::File::from_raw_fd(*fd) }
                        .write_all(&report)
                        .map_err(|e| e.to_string())
                });
            if let Err(error) = result.and(reported) {
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
    // The worker's own helpers are not part of the application's process tree.
    let mut command = crate::apps::native::application_command(
        launch
            .app
            .exec_command
            .as_deref()
            .ok_or("Desktop application has no command")?,
        launch.env.iter().cloned(),
        crate::apps::native::session_environment(),
    );
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

/// How long descendants that left the application's process group get after a
/// termination request before the reaper kills them. It is shorter than the
/// grace period of whoever stops the group, since that ends with killing the
/// group, reaper included.
const REAP_GRACE: Duration = Duration::from_secs(2);

/// Runs an application command as a child subreaper and returns once the
/// command and every process it left behind have exited. The reaper must be the
/// only thread of its process.
///
/// A termination request does not end the reaper early, which would make an
/// application that is still exiting look stopped. Whoever stops the
/// application signals its whole process group, so the reaper passes the
/// request on only to descendants outside that group: some applications, like
/// Chrome, give up a graceful exit on a second request.
pub fn reap(command: Vec<std::ffi::OsString>) -> i32 {
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } != 0 {
        eprintln!("Cannot track application processes");
        return 1;
    }
    let Some((program, args)) = command.split_first() else {
        return 2;
    };
    // Taken synchronously below; the application starts with an empty mask.
    let mut signals: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigemptyset(&mut signals);
        for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP, libc::SIGCHLD] {
            libc::sigaddset(&mut signals, signal);
        }
        libc::sigprocmask(libc::SIG_BLOCK, &signals, std::ptr::null_mut());
    }
    if let Err(e) = Command::new(program).args(args).spawn() {
        eprintln!("Cannot launch application: {e}");
        return 1;
    }
    let group = unsafe { libc::getpgrp() };
    let mut deadline: Option<Instant> = None;
    let mut terminating = false;
    loop {
        loop {
            match unsafe { libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) } {
                0 => break,
                -1 if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) => {}
                -1 => return 0,
                _ => {}
            }
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            signal_descendants(group, libc::SIGKILL);
            deadline = None;
        }
        let timeout = deadline.map(|d| {
            let left = d.saturating_duration_since(Instant::now());
            libc::timespec {
                tv_sec: left.as_secs() as libc::time_t,
                tv_nsec: left.subsec_nanos() as libc::c_long,
            }
        });
        let signal = unsafe {
            libc::sigtimedwait(
                &signals,
                std::ptr::null_mut(),
                timeout.as_ref().map_or(std::ptr::null(), |t| t as *const _),
            )
        };
        if !terminating && matches!(signal, libc::SIGTERM | libc::SIGINT | libc::SIGHUP) {
            terminating = true;
            signal_descendants(group, signal);
            deadline = Some(Instant::now() + REAP_GRACE);
        }
    }
}

/// Signals this process's descendants outside its process `group`.
fn signal_descendants(group: libc::pid_t, signal: libc::c_int) {
    // Parent and process group of every process, from /proc/<pid>/stat.
    let processes: Vec<(libc::pid_t, libc::pid_t, libc::pid_t)> = fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let pid = entry.ok()?.file_name().to_str()?.parse().ok()?;
            let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            let mut fields = stat.rsplit_once(')')?.1.split_whitespace().skip(1);
            Some((
                pid,
                fields.next()?.parse().ok()?,
                fields.next()?.parse().ok()?,
            ))
        })
        .collect();
    let mut parents = vec![unsafe { libc::getpid() }];
    while let Some(parent) = parents.pop() {
        for &(pid, ppid, pgrp) in &processes {
            if ppid == parent {
                parents.push(pid);
                if pgrp != group {
                    unsafe { libc::kill(pid, signal) };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtimes_inherit_the_environment_but_not_the_session() {
        assert!(session_env("WAYLAND_DISPLAY") && session_env("XDG_RUNTIME_DIR"));
        assert!(session_env("PULSE_SERVER") && session_env("DBUS_SESSION_BUS_ADDRESS"));
        assert!(session_env("IVNC_HOME") && session_env("IVNC_PROXY_PANEL_ENABLED"));
        assert!(!session_env("http_proxy") && !session_env("LANG") && !session_env("PATH"));
    }

    #[test]
    fn launch_description_is_removed_once_read() {
        let root = std::env::temp_dir().join(format!("ivnc-launch-{}", super::super::token()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("launch.json");
        let launch = Launch {
            app: ManagedApp::for_test("app", AppType::DesktopApp),
            config: Config::default(),
            root: root.clone(),
            generation: super::super::token(),
            env: Vec::new(),
        };
        private_write(&path, &serde_json::to_vec(&launch).unwrap()).unwrap();
        assert_eq!(read_launch(&path).unwrap().generation, launch.generation);
        assert!(!path.exists());
        assert!(read_launch(&path).is_err());
        fs::remove_dir_all(root).unwrap();
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
