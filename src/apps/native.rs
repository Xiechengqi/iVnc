use super::app::{AppType, ManagedApp};
use crate::remote::store::RemoteStore;
use log::info;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

/// Variables of the display and audio session an application joins. They are
/// the only ones that differ between the main desktop and a runtime of its own.
pub const SESSION_ENV: [&str; 3] = ["WAYLAND_DISPLAY", "XDG_RUNTIME_DIR", "PULSE_SERVER"];

/// Variables an application never takes from the manager or from its own
/// settings: its home, the manager's session and bus, and preloaded libraries.
fn reserved_env(key: &str) -> bool {
    SESSION_ENV.contains(&key)
        || crate::paths::APP_ENV_REMOVE.contains(&key)
        || matches!(key, "HOME" | "DISPLAY" | "WAYLAND_SOCKET" | "LD_PRELOAD")
        || key.starts_with("DBUS_")
}

/// The environment of a desktop application apart from its session, built from
/// the manager's environment `inherited`. It is the same whether the application
/// runs in the main desktop or in its own runtime. The manager's own settings
/// are left out; applications only receive their own configured variables.
pub fn app_environment(
    app: &ManagedApp,
    home: &Path,
    inherited: impl IntoIterator<Item = (OsString, OsString)>,
) -> BTreeMap<OsString, OsString> {
    let mut env: BTreeMap<OsString, OsString> = inherited
        .into_iter()
        .filter(|(key, _)| {
            key.to_str()
                .is_some_and(|key| !reserved_env(key) && !key.starts_with("IVNC_"))
        })
        .collect();
    for (key, value) in app.env_vars.iter().flatten() {
        if !reserved_env(key) {
            env.insert(key.into(), value.into());
        }
    }
    env.insert("HOME".into(), home.into());
    // Electron 38+ selects Ozone's native backend from the session type. Do not
    // inherit an X11 login session from the host running this compositor.
    env.insert("XDG_SESSION_TYPE".into(), "wayland".into());
    // Applications are Wayland clients; their own setting wins.
    for key in ["GDK_BACKEND", "QT_QPA_PLATFORM"] {
        env.entry(key.into()).or_insert_with(|| "wayland".into());
    }
    env
}

/// The session variables of this process, for the applications it displays.
pub fn session_environment() -> Vec<(OsString, OsString)> {
    SESSION_ENV
        .iter()
        .filter_map(|key| Some((OsString::from(key), std::env::var_os(key)?)))
        .collect()
}

/// Runs a desktop application's command under the reaper, which owns its
/// process tree: a launcher that forks and exits does not look like a crash,
/// and the application ends only when the last of its processes exits.
pub fn application_command(
    app: &ManagedApp,
    env: impl IntoIterator<Item = (OsString, OsString)>,
    session: impl IntoIterator<Item = (OsString, OsString)>,
    profile_homes: Vec<std::path::PathBuf>,
) -> Result<Command, super::error::ApiError> {
    let env: BTreeMap<OsString, OsString> = env.into_iter().collect();
    let mut plan = super::compat::plan(app, &env, unsafe { libc::geteuid() })?;
    plan.profile_homes = profile_homes;
    // A worker has its own IVNC_HOME. The app's HOME is the common durable
    // directory supplied by the manager; logs/reports live beside that HOME.
    let log_dir = env
        .get(std::ffi::OsStr::new("HOME"))
        .and_then(|p| Path::new(p).parent())
        .ok_or("Missing application HOME")?;
    plan.diagnostic_log = Some(log_dir.join("app.log"));
    plan.report_path = Some(log_dir.join("launch-info.json"));
    let mut cmd = Command::new("/proc/self/exe");
    if let Some(home) = env.get(std::ffi::OsStr::new("HOME")) {
        cmd.current_dir(home);
    }
    cmd.args(["--app-reaper", "--ivnc-launch-plan"])
        .arg(serde_json::to_string(&plan).map_err(|e| e.to_string())?)
        .env_clear()
        .envs(env)
        .envs(session);
    Ok(cmd)
}

/// Log file path for a managed app.
pub fn log_path(app_id: &str) -> std::path::PathBuf {
    let dir = crate::paths::app_log_dir(app_id);
    let _ = fs::create_dir_all(&dir);
    dir.join("app.log")
}

/// Whether a desktop app runs in its own runtime. Only apps with remote access
/// do; all others are windows of the main desktop.
pub fn runs_in_worker(app: &ManagedApp, remote: &RemoteStore) -> Result<bool, String> {
    Ok(remote.policy(&app.id)?.enabled)
}

/// Build the launch command for a desktop app, in its own runtime if `worker`.
pub fn build_command(app: &ManagedApp, worker: bool) -> Result<Command, super::error::ApiError> {
    if app.app_type != AppType::DesktopApp {
        return Err("BackgroundApp runs as a background process".into());
    }

    let mut cmd = if worker {
        crate::remote::worker::command(app)?
    } else {
        build_desktop_command(app)?
    };
    configure_app_session(&mut cmd);
    Ok(cmd)
}

/// Detach the outermost app supervisor/service from the manager's controlling
/// terminal. setsid also makes PID == PGID, preserving group-based shutdown.
/// Apply this only at the outer boundary: a worker's reaper and application
/// must remain in the supervisor's process group.
pub(super) fn configure_app_session(cmd: &mut Command) {
    cmd.stdin(Stdio::null());
    unsafe {
        cmd.pre_exec(|| {
            // The supervisor watches its parent process itself: PR_SET_PDEATHSIG
            // follows the spawning thread, and spawns may run on pooled threads.
            // A process group alone still shares the manager's controlling TTY:
            // interactive bash then suspends the entire group with SIGTTIN.
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

fn build_desktop_command(app: &ManagedApp) -> Result<Command, super::error::ApiError> {
    let exec_cmd = app
        .exec_command
        .as_ref()
        .ok_or("DesktopApp must have exec_command")?;

    info!("Desktop app '{}' launch info:", app.name);
    info!("  Command: {}", exec_cmd);
    for (key, value) in app.env_vars.iter().flatten() {
        info!("  Env: {}={}", key, value);
    }

    let home = crate::paths::prepare_app_home(&app.id)?;
    let session = session_environment();
    super::launch::validate_session(&home, &session)?;
    // The manager's HOME is the user's real home; the app's HOME is isolated.
    // Capture it here, before application_command clears the environment.
    let real_home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let profile_homes = crate::shell_profile::app_profile_homes(real_home.as_deref(), &home);
    let mut cmd = application_command(
        app,
        app_environment(app, &home, std::env::vars_os()),
        session,
        profile_homes,
    )?;

    let log_file = log_path(&app.id);
    info!("  Log file: {}", log_file.display());
    let stdout_file = crate::remote::worker::open_log(&log_file)?;
    let stderr_file = stdout_file
        .try_clone()
        .map_err(|e| format!("Failed to clone log file: {}", e))?;
    cmd.stdout(Stdio::from(stdout_file));
    cmd.stderr(Stdio::from(stderr_file));

    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_supervisor_has_a_detached_session_and_null_stdin() {
        if std::env::var_os("IVNC_SESSION_TEST_CHILD").is_some() {
            let pid = unsafe { libc::getpid() };
            assert_eq!(unsafe { libc::getsid(0) }, pid);
            assert_eq!(unsafe { libc::getpgrp() }, pid);
            assert_eq!(
                fs::read_link("/proc/self/fd/0").unwrap(),
                Path::new("/dev/null")
            );
            let tty = unsafe { libc::open(c"/dev/tty".as_ptr(), libc::O_RDONLY | libc::O_NOCTTY) };
            if tty >= 0 {
                unsafe { libc::close(tty) };
                panic!("application supervisor still has a controlling terminal");
            }
            return;
        }
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args([
            "--exact",
            "apps::native::tests::app_supervisor_has_a_detached_session_and_null_stdin",
            "--test-threads=1",
        ])
        .env("IVNC_SESSION_TEST_CHILD", "1");
        configure_app_session(&mut cmd);
        let output = cmd.output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn only_apps_with_remote_access_run_in_a_worker() {
        let remote = RemoteStore::in_memory();
        let app = ManagedApp::for_test("app", AppType::DesktopApp);
        assert!(!runs_in_worker(&app, &remote).unwrap());
        remote
            .set_policy("app", true, Some("password-one"))
            .unwrap();
        assert!(runs_in_worker(&app, &remote).unwrap());
        remote.set_policy("app", false, None).unwrap();
        assert!(!runs_in_worker(&app, &remote).unwrap());
    }

    #[test]
    fn both_modes_give_an_application_the_same_environment() {
        let mut app = ManagedApp::for_test("app", AppType::DesktopApp);
        app.env_vars = Some(
            [
                ("LANG", "zh_CN.UTF-8"),
                ("GDK_BACKEND", "x11"),
                ("HOME", "/elsewhere"),
                ("XDG_CONFIG_HOME", "/elsewhere"),
                ("PULSE_SERVER", "unix:/elsewhere"),
            ]
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .into(),
        );
        let manager = [
            ("PATH", "/usr/bin"),
            ("GTK_CSD", "0"),
            ("IVNC_HOME", "/data/ivnc"),
            ("IVNC_CHROME_DEBUG_PORT", "9333"),
            ("LD_PRELOAD", "/lib/nocsd.so"),
            ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/bus"),
            ("DISPLAY", ":0"),
            ("XDG_DATA_HOME", "/root/.local/share"),
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("WAYLAND_SOCKET", "12"),
            ("XDG_RUNTIME_DIR", "/run/user/0"),
            ("XDG_SESSION_TYPE", "x11"),
        ]
        .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        let home = Path::new("/data/ivnc/apps/app/home");
        let profile_homes = vec![std::path::PathBuf::from("/home/user"), home.to_path_buf()];
        let env = app_environment(&app, home, manager);
        // A runtime receives the environment through its launch description.
        let launch: Vec<(OsString, OsString)> = serde_json::from_slice(
            &serde_json::to_vec(&env.clone().into_iter().collect::<Vec<_>>()).unwrap(),
        )
        .unwrap();
        let session = |display: &str, dir: &str| {
            [
                ("WAYLAND_DISPLAY", display),
                ("XDG_RUNTIME_DIR", dir),
                ("PULSE_SERVER", dir),
            ]
            .map(|(k, v)| (OsString::from(k), OsString::from(v)))
        };
        let envs = |cmd: Command| -> BTreeMap<OsString, Option<OsString>> {
            cmd.get_envs()
                .filter(|(k, _)| !SESSION_ENV.iter().any(|s| k == s))
                .map(|(k, v)| (k.to_owned(), v.map(ToOwned::to_owned)))
                .collect()
        };
        let main = application_command(
            &app,
            env.clone(),
            session("wayland-0", "/run/user/0"),
            profile_homes.clone(),
        )
        .unwrap();
        assert_eq!(main.get_current_dir(), Some(home));
        let plan: super::super::compat::Plan =
            serde_json::from_str(main.get_args().nth(2).unwrap().to_str().unwrap()).unwrap();
        assert_eq!(
            plan.diagnostic_log,
            Some(home.parent().unwrap().join("app.log"))
        );
        assert_eq!(
            plan.report_path,
            Some(home.parent().unwrap().join("launch-info.json"))
        );
        assert_eq!(plan.profile_homes, profile_homes);
        let runtime = application_command(
            &app,
            launch,
            session("wayland-1", "/tmp/ivnc-1"),
            profile_homes,
        )
        .unwrap();
        assert_eq!(main.get_envs().count(), runtime.get_envs().count());
        assert_eq!(envs(main), envs(runtime));

        let get = |key: &str| env.get(&OsString::from(key)).and_then(|v| v.to_str());
        assert_eq!(get("HOME"), home.to_str());
        assert_eq!(get("LANG"), Some("zh_CN.UTF-8"));
        assert_eq!(get("PATH"), Some("/usr/bin"));
        // The application's own settings win over defaults and the manager.
        assert_eq!(get("GDK_BACKEND"), Some("x11"));
        assert_eq!(get("QT_QPA_PLATFORM"), Some("wayland"));
        assert_eq!(get("XDG_SESSION_TYPE"), Some("wayland"));
        for key in [
            "IVNC_HOME",
            "IVNC_PROXY_PANEL_ENABLED",
            "IVNC_CHROME_DEBUG_PORT",
            "LD_PRELOAD",
            "DBUS_SESSION_BUS_ADDRESS",
            "DISPLAY",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "WAYLAND_DISPLAY",
            "WAYLAND_SOCKET",
            "XDG_RUNTIME_DIR",
            "PULSE_SERVER",
        ] {
            assert_eq!(get(key), None, "{key}");
        }
        // Manager IVNC_ variables are never forwarded to applications.
        app.env_vars = None;
        let env = app_environment(
            &app,
            home,
            [("IVNC_PROXY_PANEL_ENABLED".into(), "0".into())],
        );
        assert_eq!(env.get(&OsString::from("IVNC_PROXY_PANEL_ENABLED")), None);
    }
}
