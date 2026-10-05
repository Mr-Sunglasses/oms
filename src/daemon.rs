//! The background agent (a `LaunchAgent` running `oms daemon`). It switches the
//! wallpaper and app themes when macOS changes between light and dark, and
//! rotates wallpapers. Ghostty switches its own theme (`theme = light:…,dark:…`).

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2_foundation::{
    NSDate, NSDefaultRunLoopMode, NSDistributedNotificationCenter, NSNotification, NSRunLoop,
    NSString,
};

use anyhow::{Context, Result};

use crate::repo::{Repos, Theme};
use crate::settings::{Choice, Settings, data_dir};
use crate::{actions, ghostty, wallpaper};

const LABEL: &str = "xyz.kanishkk.oms";

/// The agent's launchd name. A custom `OMS_DATA_DIR` gets its own agent, so a
/// second setup (or a test) never replaces or removes the main one.
pub fn label() -> String {
    match std::env::var_os("OMS_DATA_DIR") {
        Some(dir) => {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            dir.hash(&mut hasher);
            format!("{LABEL}.{:08x}", hasher.finish() as u32)
        }
        None => LABEL.to_string(),
    }
}
/// Posted by oms whenever its settings change.
const SETTINGS_CHANGED: &str = "xyz.kanishkk.oms.settings-changed";
/// Posted by macOS when it switches between light and dark mode.
const APPEARANCE_CHANGED: &str = "AppleInterfaceThemeChangedNotification";
/// A check now and then even without notifications, in case one was missed.
const SAFETY_TICK: Duration = Duration::from_secs(300);

fn plist_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", label()))
}

fn domain() -> String {
    let uid = Command::new("id")
        .arg("-u")
        .output()
        .map(|o| o.stdout)
        .unwrap_or_default();
    format!("gui/{}", String::from_utf8_lossy(&uid).trim())
}

pub fn is_installed() -> bool {
    plist_path().exists()
}

/// Installs or removes the agent to match the settings.
pub fn sync(settings: &Settings) -> Result<()> {
    if settings.needs_daemon() {
        install()
    } else {
        uninstall()
    }
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

fn install() -> Result<()> {
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

fn log(message: &str) {
    let out = Command::new("date")
        .arg("+%F %T")
        .output()
        .map(|o| o.stdout)
        .unwrap_or_default();
    eprintln!("{} {message}", String::from_utf8_lossy(&out).trim());
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

/// Sets `flag` whenever macOS changes appearance or oms changes its settings.
fn observe(flag: &Arc<AtomicBool>) -> Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>> {
    let center = NSDistributedNotificationCenter::defaultCenter();
    [APPEARANCE_CHANGED, SETTINGS_CHANGED]
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
        .collect()
}

/// Sleeps on the run loop until a notification arrives or `timeout` passes.
fn wait(timeout: Duration, flag: &AtomicBool) {
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

/// The agent's loop. Runs until launchd stops it. It sleeps until macOS
/// changes appearance, oms changes its settings, or a wallpaper is due.
pub fn run() -> Result<()> {
    let repos = Repos::locate(None, None)?;
    let woken = Arc::new(AtomicBool::new(true));
    let _observers = observe(&woken);
    // What the agent last switched to, so a new light/dark pick applies at once.
    let mut last: Option<Choice> = None;
    let mut last_rotation = Instant::now();
    let mut last_check = Instant::now();
    let mut settings = Settings::default();
    let mut dark = false;
    log("started");
    loop {
        if woken.swap(false, Ordering::SeqCst) || last_check.elapsed() >= SAFETY_TICK {
            last_check = Instant::now();
            settings = Settings::load();
            dark = is_dark();
            if settings.auto
                && let (Some(light), Some(dark_choice)) = (&settings.light, &settings.dark)
            {
                let choice = if dark { dark_choice } else { light };
                if last.as_ref() != Some(choice) {
                    let themes = repos.load(&settings).unwrap_or_default();
                    if let Err(e) = follow_appearance(&themes, choice, &settings) {
                        log(&format!("couldn't switch to {}: {e:#}", choice.theme));
                    }
                    last = Some(choice.clone());
                    last_rotation = Instant::now();
                }
            } else {
                last = None;
            }
        }

        let mut timeout = SAFETY_TICK;
        if let Some(minutes) = settings.rotate_minutes {
            let every = Duration::from_secs(minutes.max(1) * 60);
            if last_rotation.elapsed() >= every {
                let themes = repos.load(&settings).unwrap_or_default();
                if let Err(e) = rotate(&themes, &settings, dark) {
                    log(&format!("couldn't rotate: {e:#}"));
                }
                last_rotation = Instant::now();
                settings = Settings::load();
            }
            timeout = timeout.min(every.saturating_sub(last_rotation.elapsed()));
        }
        wait(timeout.max(Duration::from_millis(100)), &woken);
    }
}

fn follow_appearance(themes: &[Theme], choice: &Choice, settings: &Settings) -> Result<()> {
    let theme = actions::find(themes, &choice.theme)?;
    let path = theme
        .wallpapers
        .get(choice.wallpaper)
        .or_else(|| theme.wallpapers.first());
    if let Some(path) = path
        && wallpaper::current().as_ref() != Some(path)
    {
        wallpaper::set(path)?;
    }
    let apps = actions::apply_apps(theme, settings);
    log(&format!("switched to {}{apps}", theme.name));
    Ok(())
}

/// The theme in use now: the light/dark one, or Ghostty's single theme.
fn active_theme<'a>(themes: &'a [Theme], settings: &Settings, dark: bool) -> Option<&'a Theme> {
    if settings.auto {
        let choice = if dark {
            &settings.dark
        } else {
            &settings.light
        };
        if let Some(choice) = choice {
            return actions::find(themes, &choice.theme).ok();
        }
    }
    let current = ghostty::current_theme(&ghostty::config_path())?;
    themes.iter().find(|t| t.ghostty_name == current)
}

fn rotate(themes: &[Theme], settings: &Settings, dark: bool) -> Result<()> {
    let theme = active_theme(themes, settings, dark).context("no Omarchy theme in use")?;
    let count = theme.wallpapers.len();
    if count < 2 {
        return Ok(());
    }
    let index = (settings
        .wallpaper_index
        .get(&theme.slug)
        .copied()
        .unwrap_or(0)
        + 1)
        % count;
    // Reload so a change made in the meantime isn't overwritten.
    let mut fresh = Settings::load();
    let path = actions::apply_wallpaper(theme, index, &mut fresh)?;
    log(&format!("rotated to {}", path.display()));
    Ok(())
}
