//! Optional loading of the user's shell profile files before managed commands.
//!
//! iVnc normally builds a minimal, deterministic environment and never runs
//! shell startup files. This module implements the global opt-out: source
//! `$HOME/.profile` and `$HOME/.bashrc` from the manager user's real HOME first,
//! then from the application's isolated HOME, and restore the session and
//! isolation variables iVnc owns before handing control to the original
//! command.
//!
//! The real HOME is a property of the *manager* process. A worker supervisor
//! already has the application's isolated HOME in its environment, so the
//! manager captures the real one and carries it in the launch description.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

static APPS: OnceLock<bool> = OnceLock::new();
static TERMINAL: OnceLock<bool> = OnceLock::new();

/// Install the process-wide switches from the loaded configuration. Values
/// default to `true` (global default on) until this is called.
pub fn configure(config: &crate::config::Config) {
    let _ = APPS.set(config.shell_profile.apps);
    let _ = TERMINAL.set(config.shell_profile.terminal);
}

pub fn apps_enabled() -> bool {
    *APPS.get().unwrap_or(&true)
}

pub fn terminal_enabled() -> bool {
    *TERMINAL.get().unwrap_or(&true)
}

fn usable(home: &Path) -> bool {
    home.is_absolute() && !home.as_os_str().is_empty()
}

/// Ordered homes to read profile files from. The real user home comes first;
/// the application's isolated home supplements it. Duplicate paths are removed
/// and the list is empty when profile loading is disabled.
pub fn app_profile_homes(real_home: Option<&Path>, app_home: &Path) -> Vec<PathBuf> {
    if !apps_enabled() {
        return Vec::new();
    }
    let mut homes = Vec::new();
    if let Some(home) = real_home.filter(|home| usable(home)) {
        homes.push(home.to_path_buf());
    }
    if usable(app_home) && !homes.iter().any(|home| home == app_home) {
        homes.push(app_home.to_path_buf());
    }
    homes
}

/// Shell snippet that sources the two profile files for each home. Used by the
/// web terminal's `--rcfile` / `ENV` so functions and aliases loaded from rc
/// stay in the same shell that the user interacts with.
pub fn terminal_profile_script(homes: &[PathBuf]) -> String {
    let mut script = String::from("# iVnc terminal profile bootstrap\n");
    for home in homes.iter().filter(|home| usable(home)) {
        let home = shell_quote(home);
        script.push_str(&format!(
            "if [ -r {home}/.profile ]; then . {home}/.profile; fi\n\
             if [ -r {home}/.bashrc ]; then . {home}/.bashrc; fi\n"
        ));
    }
    script
}

/// Build the command that boots an application command through bash:
///
/// - `--noprofile --norc` keeps bash from reading files on its own, so the
///   order below is fully controlled (real home first, isolated home second).
/// - `-i` sets the interactive flag in `$-`, which is what Debian's default
///   `~/.bashrc` checks before it exports anything. Without it a sourced
///   `.bashrc` would just `return`.
/// - The original command is `$0` (passed as a single argument, never parsed)
///   and the homes are `$@`. While each home's files are sourced `HOME` is
///   temporarily set to that home, so rc files that expand `$HOME/.nvm` (nvm,
///   pyenv, …) resolve against the home they belong to. After sourcing, the
///   variables iVnc owns are re-applied so a user's rc cannot switch the app to
///   X11 or point it at the host's session bus.
/// - The original command is finally `eval`-ed in this same bash, not handed to
///   `/bin/sh`. That keeps the loaded PATH and makes bash-only builtins such as
///   `source` usable in the launch command itself (dash has no `source`).
pub fn app_command(original: &str, homes: &[PathBuf]) -> Command {
    const SCRIPT: &str = r#"
ivnc_app_home=$HOME
ivnc_runtime=${XDG_RUNTIME_DIR-}
ivnc_display=${WAYLAND_DISPLAY-}
ivnc_pulse=${PULSE_SERVER-}
for ivnc_h in "$@"; do
  [ -n "$ivnc_h" ] || continue
  HOME="$ivnc_h"
  if [ -r "$ivnc_h/.profile" ]; then . "$ivnc_h/.profile"; fi
  if [ -r "$ivnc_h/.bashrc" ]; then . "$ivnc_h/.bashrc"; fi
done
export HOME="$ivnc_app_home" XDG_SESSION_TYPE=wayland GDK_BACKEND=wayland QT_QPA_PLATFORM=wayland
export XDG_RUNTIME_DIR="$ivnc_runtime" WAYLAND_DISPLAY="$ivnc_display" PULSE_SERVER="$ivnc_pulse"
unset DISPLAY WAYLAND_SOCKET LD_PRELOAD
cd "$ivnc_app_home" 2>/dev/null || :
eval "$0"
"#;
    // The reaper's PATH comes from the filtered application environment; use
    // an absolute bash path when available so the bootstrap cannot go missing.
    let shell = ["/bin/bash", "/usr/bin/bash"]
        .into_iter()
        .find(|path| Path::new(path).exists())
        .unwrap_or("bash");
    let mut command = Command::new(shell);
    command.args(["--noprofile", "--norc", "-i", "-c", SCRIPT]);
    command.arg(original);
    for home in homes {
        command.arg(home);
    }
    command
}

/// Write the terminal's rc script where the spawned shell can read it. The
/// path is stable for this process so no per-session temp file is needed.
pub fn write_terminal_profile(homes: &[PathBuf]) -> Result<PathBuf, String> {
    let dir = crate::paths::ivnc_home();
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let path = dir.join(format!("terminal-profile-{}.sh", std::process::id()));
    std::fs::write(&path, terminal_profile_script(homes)).map_err(|error| error.to_string())?;
    Ok(path)
}

fn shell_quote(value: &Path) -> String {
    let value = value.to_string_lossy();
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(path: &str) -> PathBuf {
        PathBuf::from(path)
    }

    #[test]
    fn homes_are_ordered_real_first_and_deduplicated() {
        let homes = app_profile_homes(
            Some(Path::new("/home/user")),
            Path::new("/data/apps/a/home"),
        );
        assert_eq!(homes, [home("/home/user"), home("/data/apps/a/home")]);
        let same = app_profile_homes(Some(Path::new("/home/user")), Path::new("/home/user"));
        assert_eq!(same, [home("/home/user")]);
        let none = app_profile_homes(None, Path::new("/data/apps/a/home"));
        assert_eq!(none, [home("/data/apps/a/home")]);
        assert!(app_profile_homes(None, Path::new("relative")).is_empty());
    }

    #[test]
    fn app_command_passes_command_as_zero_and_homes_as_arguments() {
        let command = app_command(
            "exec google-chrome --flag",
            &[home("/home/user"), home("/data/apps/a/home")],
        );
        assert!(command.get_program().to_string_lossy().ends_with("/bash"));
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args[0], "--noprofile");
        assert_eq!(args[1], "--norc");
        assert_eq!(args[2], "-i");
        assert_eq!(args[3], "-c");
        let script = args[4].to_str().unwrap();
        assert!(script.contains("/.profile"));
        assert!(script.contains("/.bashrc"));
        assert!(script.contains("XDG_SESSION_TYPE=wayland"));
        assert!(script.contains("HOME=\"$ivnc_h\""));
        assert!(script.contains("eval \"$0\""));
        assert_eq!(args[5], "exec google-chrome --flag");
        assert_eq!(args[6], "/home/user");
        assert_eq!(args[7], "/data/apps/a/home");
    }

    #[test]
    fn command_text_is_never_reparsed_by_the_wrapper() {
        // Quotes, semicolons and newlines in the original command stay a
        // single $0 argument and are only interpreted by /bin/sh -c.
        let original = "echo 'a;b'\nprintf x";
        let command = app_command(original, &[]);
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args[5], original);
        assert_eq!(args.len(), 6);
    }

    #[test]
    fn bootstrap_expands_home_per_file_and_allows_source_in_command() {
        let root = std::env::temp_dir().join(format!("ivnc-profile-home-{}", std::process::id()));
        let real = root.join("real");
        let app = root.join("app");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::create_dir_all(&app).unwrap();
        // NVM_DIR-style expansion must resolve against the home being sourced,
        // not the app's isolated HOME passed in the environment.
        std::fs::write(real.join(".bashrc"), "export REAL_HOME_SEEN=$HOME\n").unwrap();
        std::fs::write(app.join(".bashrc"), "export APP_HOME_SEEN=$HOME\n").unwrap();
        std::fs::write(real.join("extra.sh"), "export SOURCE_WORKED=1\n").unwrap();
        let out = root.join("result");
        // The launch command itself uses `source`, which dash does not have.
        let command = format!(
            "source '{}'; printf '%s|%s|%s|%s' \"$REAL_HOME_SEEN\" \"$APP_HOME_SEEN\" \"$SOURCE_WORKED\" \"$HOME\" > '{}'",
            real.join("extra.sh").display(),
            out.display()
        );

        let status = app_command(&command, &[real.clone(), app.clone()])
            .env("HOME", &app) // the app's isolated HOME, as set in real launches
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        let result = std::fs::read_to_string(&out).unwrap();
        assert_eq!(
            result,
            format!("{}|{}|1|{}", real.display(), app.display(), app.display())
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn bootstrap_loads_guarded_bashrc_and_restores_isolation() {
        let root = std::env::temp_dir().join(format!("ivnc-profile-{}", std::process::id()));
        let real = root.join("real");
        let app = root.join("app");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(
            real.join(".profile"),
            "export FROM_PROFILE=real; export PATH=/usr/bin:/bin\n",
        )
        .unwrap();
        std::fs::write(
            real.join(".bashrc"),
            "case $- in *i*) ;; *) return;; esac\n\
             export FROM_BASHRC=real\n\
             export XDG_SESSION_TYPE=x11\n\
             export DISPLAY=:0\n",
        )
        .unwrap();
        std::fs::write(app.join(".profile"), "export FROM_APP_PROFILE=app\n").unwrap();
        let out = root.join("result");
        let command = format!(
            "printf '%s|%s|%s|%s|%s' \"$FROM_PROFILE\" \"$FROM_BASHRC\" \"$FROM_APP_PROFILE\" \"$XDG_SESSION_TYPE\" \"${{DISPLAY-unset}}\" > '{}'",
            out.display()
        );

        let status = app_command(&command, &[real.clone(), app.clone()])
            .env("HOME", &app)
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            std::fs::read_to_string(&out).unwrap(),
            "real|real|app|wayland|unset"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn terminal_script_sources_both_files_per_home_in_order() {
        let script = terminal_profile_script(&[home("/home/user"), home("/data/apps/a/home")]);
        let first_profile = script.find("'/home/user'/.profile").unwrap();
        let first_bashrc = script.find("'/home/user'/.bashrc").unwrap();
        let second_profile = script.find("'/data/apps/a/home'/.profile").unwrap();
        assert!(first_profile < first_bashrc);
        assert!(first_bashrc < second_profile);
        assert!(script.contains("'/data/apps/a/home'/.bashrc"));
    }
}
