//! macOS: the agent is a `LaunchAgent`; light/dark and settings changes arrive
//! as distributed notifications.

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2_foundation::{
    NSDate, NSDefaultRunLoopMode, NSDistributedNotificationCenter, NSNotification, NSRunLoop,
    NSString,
};

use super::{OtherAgent, label, other_label};
use crate::settings::data_dir;

/// Posted by oms whenever its settings change.
const SETTINGS_CHANGED: &str = "xyz.kanishkk.oms.settings-changed";
/// Posted by macOS when it switches between light and dark mode.
const APPEARANCE_CHANGED: &str = "AppleInterfaceThemeChangedNotification";

fn plist_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", label()))
}

/// launchd's domain for this user's agents.
fn domain() -> String {
    format!("gui/{}", crate::clock::uid())
}

pub fn is_installed() -> bool {
    plist_path().exists()
}

/// Whether launchd has the agent loaded and running.
pub fn is_running() -> bool {
    Command::new("launchctl")
        .args(["print", &format!("{}/{}", domain(), label())])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("state = running"))
}

/// Every oms agent in `~/Library/LaunchAgents` except this setup's own.
pub fn other_agents() -> Vec<OtherAgent> {
    let Some(dir) = plist_path().parent().map(PathBuf::from) else {
        return Vec::new();
    };
    let own = label();
    let mut agents: Vec<OtherAgent> = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let label = other_label(&name, ".plist", &own)?;
            let file = entry.path();
            let program = plist::Value::from_file(&file).ok().and_then(|v| {
                let args = v.as_dictionary()?.get("ProgramArguments")?.as_array()?;
                Some(PathBuf::from(args.first()?.as_string()?))
            });
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

/// Stops and removes another oms agent.
pub fn remove_other(agent: &OtherAgent) -> Result<()> {
    let _ = Command::new("launchctl")
        .args(["bootout", &format!("{}/{}", domain(), agent.label)])
        .output();
    fs::remove_file(&agent.file)?;
    Ok(())
}

/// The shell commands that remove another agent by hand.
pub fn remove_hint(agent: &OtherAgent) -> String {
    format!(
        "launchctl bootout {}/{}; rm '{}'",
        domain(),
        agent.label,
        agent.file.display()
    )
}

fn plist() -> Result<String> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let log = data_dir().join("agent.log");
    let esc = |s: String| s.replace('&', "&amp;").replace('<', "&lt;");
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{}</string>
  <key>ProgramArguments</key>
  <array><string>{}</string><string>daemon</string></array>
{}  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ProcessType</key><string>Background</string>
  <key>StandardOutPath</key><string>{}</string>
  <key>StandardErrorPath</key><string>{}</string>
</dict>
</plist>
"#,
        label(),
        esc(exe.display().to_string()),
        environment(),
        esc(log.display().to_string()),
        esc(log.display().to_string()),
    ))
}

/// Passes on PATH (launchd's default leaves out Homebrew, so bat or tmux
/// wouldn't be found) and settings that change where files live.
fn environment() -> String {
    let path = std::env::var("PATH").unwrap_or_default();
    let path = if path.contains("/opt/homebrew/bin") {
        path
    } else {
        format!("/opt/homebrew/bin:/usr/local/bin:{path}")
    };
    let mut vars: Vec<(&str, String)> = [
        "XDG_CONFIG_HOME",
        "OMS_DATA_DIR",
        "BAT_CONFIG_DIR",
        "BAT_CACHE_PATH",
    ]
    .iter()
    .filter_map(|k| std::env::var(k).ok().map(|v| (*k, v)))
    .collect();
    vars.push(("PATH", path));
    let entries = vars.iter().fold(String::new(), |mut out, (k, v)| {
        let v = v.replace('&', "&amp;").replace('<', "&lt;");
        let _ = writeln!(out, "    <key>{k}</key><string>{v}</string>");
        out
    });
    format!("  <key>EnvironmentVariables</key>\n  <dict>\n{entries}  </dict>\n")
}

pub fn install() -> Result<()> {
    let path = plist_path();
    let text = plist()?;
    let loaded = Command::new("launchctl")
        .args(["print", &format!("{}/{}", domain(), label())])
        .output()
        .is_ok_and(|o| o.status.success());
    if loaded && fs::read_to_string(&path).is_ok_and(|t| t == text) {
        return Ok(());
    }
    fs::create_dir_all(path.parent().context("bad LaunchAgents path")?)?;
    fs::create_dir_all(data_dir())?;
    fs::write(&path, text)?;
    let _ = Command::new("launchctl")
        .args(["bootout", &format!("{}/{}", domain(), label())])
        .output();
    let out = Command::new("launchctl")
        .args(["bootstrap", &domain()])
        .arg(&path)
        .output()?;
    if !out.status.success() {
        anyhow::bail!(
            "launchctl bootstrap failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Restarts the agent (e.g. after an update) so it runs the new binary.
pub fn restart_if_running() {
    if is_installed() {
        let _ = Command::new("launchctl")
            .args(["kickstart", "-k", &format!("{}/{}", domain(), label())])
            .output();
    }
}

pub fn uninstall() -> Result<()> {
    let path = plist_path();
    if path.exists() {
        let _ = Command::new("launchctl")
            .args(["bootout", &format!("{}/{}", domain(), label())])
            .output();
        fs::remove_file(&path)?;
    }
    Ok(())
}

pub fn is_dark() -> bool {
    Command::new("defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "Dark")
}

/// Tells the agent (if it's running) to reread the settings.
pub fn post_settings_changed() {
    let center = NSDistributedNotificationCenter::defaultCenter();
    // SAFETY: a valid notification name with no object or user info, which the API allows.
    unsafe {
        center.postNotificationName_object_userInfo_deliverImmediately(
            &NSString::from_str(SETTINGS_CHANGED),
            None,
            None,
            true,
        );
    }
}

/// Sets the agent's wake flag whenever macOS changes appearance or oms changes
/// its settings. The observers stay registered while this lives.
pub struct Watcher {
    _observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
}

impl Watcher {
    pub fn new(flag: &Arc<AtomicBool>) -> Self {
        let center = NSDistributedNotificationCenter::defaultCenter();
        let observers = [APPEARANCE_CHANGED, SETTINGS_CHANGED]
            .iter()
            .map(|name| {
                let flag = flag.clone();
                let block = RcBlock::new(move |_: NonNull<NSNotification>| {
                    flag.store(true, Ordering::SeqCst);
                });
                // SAFETY: the block only stores to an `AtomicBool`, so it is fine to run
                // on whatever thread delivers the notification (the queue is nil).
                unsafe {
                    center.addObserverForName_object_queue_usingBlock(
                        Some(&NSString::from_str(name)),
                        None,
                        None,
                        &block,
                    )
                }
            })
            .collect();
        Self {
            _observers: observers,
        }
    }

    /// Sleeps on the run loop until a notification arrives or `timeout`
    /// passes. Notifications cover appearance changes, so `_auto` isn't needed.
    // Same signature as the Linux watcher, which keeps state between waits.
    #[allow(clippy::unused_self)]
    pub fn wait(&mut self, timeout: Duration, flag: &AtomicBool, _auto: bool) {
        let started = Instant::now();
        let until = NSDate::dateWithTimeIntervalSinceNow(timeout.as_secs_f64());
        let run_loop = NSRunLoop::currentRunLoop();
        // SAFETY: `NSDefaultRunLoopMode` is a constant string exported by Foundation;
        // reading the extern static is all that's unsafe here.
        unsafe { run_loop.runMode_beforeDate(NSDefaultRunLoopMode, &until) };
        // Guard against a run loop with nothing to wait on returning at once.
        if !flag.load(Ordering::SeqCst) && started.elapsed() < Duration::from_millis(10) {
            std::thread::sleep(timeout.min(Duration::from_secs(1)));
        }
    }
}
