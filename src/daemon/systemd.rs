//! Linux: the agent is a systemd user service. There are no system-wide
//! notifications to listen to, so the agent polls cheaply instead: the settings
//! file's modification time every second, and the light/dark mode every few
//! seconds while light/dark switching is on.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, bail};

pub use super::linux_appearance::is_dark;
use super::{OtherAgent, label, other_label};
use crate::settings::data_dir;

/// Variables the agent needs to reach the desktop session (to set the
/// wallpaper and read light/dark) and to find files where you keep them. The
/// unit records only those systemd's user manager doesn't already have: most
/// desktops hand it their session's values at each login, and a recorded
/// copy (an Xwayland auth file, say) would go stale after the next one.
const PASSED_ENV: &[&str] = &[
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "DBUS_SESSION_BUS_ADDRESS",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_DESKTOP",
    "XDG_SESSION_TYPE",
    "DESKTOP_SESSION",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "OMS_DATA_DIR",
    "OMS_WALLPAPER_BACKEND",
    "BAT_CONFIG_DIR",
    "BAT_CACHE_PATH",
    "PATH",
];

/// How often to recheck light/dark mode while it's being followed.
const APPEARANCE_POLL: Duration = Duration::from_secs(3);

/// systemd's user manager environment (`systemctl --user show-environment`).
fn manager_env() -> &'static str {
    static ENV: OnceLock<String> = OnceLock::new();
    ENV.get_or_init(|| systemctl(&["show-environment"]).unwrap_or_default())
}

/// A variable's value in `show-environment` output.
fn env_value<'a>(env: &'a str, key: &str) -> Option<&'a str> {
    env.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
}

/// The user unit folder systemd reads: its own `XDG_CONFIG_HOME` (which can
/// differ from this shell's), else `~/.config`.
fn unit_dir() -> PathBuf {
    env_value(manager_env(), "XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map_or_else(
            || dirs::home_dir().unwrap_or_default().join(".config"),
            PathBuf::from,
        )
        .join("systemd/user")
}

/// The variables from `current` that the manager (`show-environment` output)
/// lacks or has a different value for.
fn env_to_record<'a>(current: &[(&'a str, String)], manager: &str) -> Vec<(&'a str, String)> {
    current
        .iter()
        .filter(|(key, value)| env_value(manager, key) != Some(value.as_str()))
        .cloned()
        .collect()
}

fn unit_name() -> String {
    format!("{}.service", label())
}

fn unit_path() -> PathBuf {
    unit_dir().join(unit_name())
}

/// Runs `systemctl --user <args>`; Ok if it exited successfully.
fn systemctl(args: &[&str]) -> Result<String> {
    let out = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()
        .context("running systemctl (oms needs systemd for light/dark switching and rotation)")?;
    if !out.status.success() {
        bail!(
            "systemctl --user {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn is_installed() -> bool {
    unit_path().exists()
}

pub fn is_running() -> bool {
    systemctl(&["is-active", &unit_name()]).is_ok_and(|s| s == "active")
}

/// Every oms unit in the user unit folder except this setup's own.
pub fn other_agents() -> Vec<OtherAgent> {
    let own = label();
    let mut agents: Vec<OtherAgent> = fs::read_dir(unit_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let label = other_label(&name, ".service", &own)?;
            let file = entry.path();
            let program = fs::read_to_string(&file)
                .ok()
                .and_then(|text| exec_program(&text));
            Some(OtherAgent {
                label,
                file,
                program,
            })
        })
        .collect();
    agents.sort_by(|a, b| a.label.cmp(&b.label));
    agents
}

/// The binary in a unit's `ExecStart=` line.
fn exec_program(unit: &str) -> Option<PathBuf> {
    let line = unit
        .lines()
        .find_map(|l| l.trim().strip_prefix("ExecStart="))?;
    let program = match line.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?,
        None => line.split_whitespace().next()?,
    };
    Some(PathBuf::from(program))
}

/// Stops and removes another oms agent.
pub fn remove_other(agent: &OtherAgent) -> Result<()> {
    let name = format!("{}.service", agent.label);
    let _ = systemctl(&["disable", "--now", &name]);
    fs::remove_file(&agent.file)?;
    let _ = systemctl(&["daemon-reload"]);
    Ok(())
}

/// The shell commands that remove another agent by hand.
pub fn remove_hint(agent: &OtherAgent) -> String {
    format!(
        "systemctl --user disable --now {}.service; rm '{}'",
        agent.label,
        agent.file.display()
    )
}

/// Quotes a value for a unit file: `%` is a specifier, `"` and `\` escape.
fn quote(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    format!("\"{escaped}\"")
}

/// The unit text for `exe`, logging to `log`, with `env` passed on.
fn unit(exe: &Path, log: &Path, env: &[(&str, String)]) -> String {
    let mut text = String::from(
        "[Unit]\nDescription=oms: Omarchy theme light/dark switching and wallpaper rotation\n\n[Service]\nType=simple\n",
    );
    let _ = writeln!(
        text,
        "ExecStart={} daemon",
        quote(&exe.display().to_string())
    );
    for (key, value) in env {
        let _ = writeln!(text, "Environment={}", quote(&format!("{key}={value}")));
    }
    let log = log.display().to_string().replace('%', "%%");
    let _ = writeln!(text, "StandardOutput=append:{log}");
    let _ = writeln!(text, "StandardError=append:{log}");
    text.push_str("Restart=on-failure\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n");
    text
}

fn current_unit() -> Result<String> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let current: Vec<(&str, String)> = PASSED_ENV
        .iter()
        .filter_map(|k| std::env::var(k).ok().map(|v| (*k, v)))
        .collect();
    let env = env_to_record(&current, manager_env());
    Ok(unit(&exe, &data_dir().join("agent.log"), &env))
}

pub fn install() -> Result<()> {
    let path = unit_path();
    let text = current_unit()?;
    let unchanged = fs::read_to_string(&path).is_ok_and(|t| t == text);
    if unchanged && is_running() {
        return Ok(());
    }
    fs::create_dir_all(unit_dir())?;
    fs::create_dir_all(data_dir())?;
    fs::write(&path, text)?;
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", &unit_name()])?;
    // Restart rather than start, so a changed unit runs with its new settings.
    systemctl(&["restart", &unit_name()])?;
    Ok(())
}

/// Restarts the agent (e.g. after an update) so it runs the new binary.
pub fn restart_if_running() {
    if is_installed() {
        let _ = systemctl(&["try-restart", &unit_name()]);
    }
}

pub fn uninstall() -> Result<()> {
    let path = unit_path();
    if path.exists() {
        let _ = systemctl(&["disable", "--now", &unit_name()]);
        fs::remove_file(&path)?;
        let _ = systemctl(&["daemon-reload"]);
    }
    Ok(())
}

/// The agent notices settings changes by the file's modification time, so
/// there's nothing to send.
pub fn post_settings_changed() {}

fn settings_mtime() -> Option<SystemTime> {
    fs::metadata(data_dir().join("settings.json"))
        .and_then(|m| m.modified())
        .ok()
}

/// Wakes the agent when the settings file changes or, while light/dark
/// switching is on, when the system's mode flips.
pub struct Watcher {
    settings: Option<SystemTime>,
    dark: bool,
    last_poll: Instant,
}

impl Watcher {
    pub fn new(_flag: &Arc<AtomicBool>) -> Self {
        Self {
            settings: settings_mtime(),
            dark: is_dark(),
            last_poll: Instant::now(),
        }
    }

    /// Sleeps up to `timeout`, a second at a time, returning early (with
    /// `flag` set) when something changed.
    pub fn wait(&mut self, timeout: Duration, flag: &AtomicBool, auto: bool) {
        let until = Instant::now() + timeout;
        loop {
            let now = Instant::now();
            if now >= until || flag.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep((until - now).min(Duration::from_secs(1)));
            let mtime = settings_mtime();
            if mtime != self.settings {
                self.settings = mtime;
                flag.store(true, Ordering::SeqCst);
            }
            if auto && self.last_poll.elapsed() >= APPEARANCE_POLL {
                self.last_poll = Instant::now();
                let dark = is_dark();
                if dark != self.dark {
                    self.dark = dark;
                    flag.store(true, Ordering::SeqCst);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_unit_that_runs_the_daemon() {
        let text = unit(
            Path::new("/home/me/.local/bin/oms"),
            Path::new("/home/me/.local/share/omarchy-switch/agent.log"),
            &[
                ("DISPLAY", ":0".to_string()),
                ("PATH", "/usr/bin:/bin".to_string()),
            ],
        );
        assert!(text.contains("ExecStart=\"/home/me/.local/bin/oms\" daemon\n"));
        assert!(text.contains("Environment=\"DISPLAY=:0\"\n"));
        assert!(
            text.contains("StandardError=append:/home/me/.local/share/omarchy-switch/agent.log\n")
        );
        assert!(text.contains("WantedBy=default.target"));
        assert_eq!(
            exec_program(&text).as_deref(),
            Some(Path::new("/home/me/.local/bin/oms"))
        );
    }

    #[test]
    fn escapes_unit_values() {
        assert_eq!(quote(r#"a "b" 100% c\d"#), r#""a \"b\" 100%% c\\d""#);
        assert_eq!(
            exec_program("[Service]\nExecStart=/usr/bin/oms daemon\n").as_deref(),
            Some(Path::new("/usr/bin/oms"))
        );
    }

    #[test]
    fn reads_the_managers_environment() {
        let env = "HOME=/home/me\nXDG_CONFIG_HOME=/home/me/.cfg\nXDG_CONFIG_HOME_X=no\n";
        assert_eq!(env_value(env, "XDG_CONFIG_HOME"), Some("/home/me/.cfg"));
        assert_eq!(env_value(env, "DISPLAY"), None);
    }

    #[test]
    fn records_only_what_systemd_lacks() {
        let current = [
            ("DISPLAY", ":0".to_string()),
            (
                "XAUTHORITY",
                "/run/user/1000/.mutter-Xwaylandauth.AB12".to_string(),
            ),
            ("PATH", "/home/me/.local/bin:/usr/bin".to_string()),
        ];
        let manager =
            "DISPLAY=:0\nXAUTHORITY=/run/user/1000/.mutter-Xwaylandauth.AB12\nPATH=/usr/bin\n";
        assert_eq!(
            env_to_record(&current, manager),
            [("PATH", "/home/me/.local/bin:/usr/bin".to_string())]
        );
        assert_eq!(env_to_record(&current, "").len(), 3);
    }
}
