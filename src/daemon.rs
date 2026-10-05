//! The background agent (a LaunchAgent running `oms daemon`). It switches the
//! wallpaper and app themes when macOS changes between light and dark, and
//! rotates wallpapers. Ghostty switches its own theme (`theme = light:…,dark:…`).

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::repo::{Repos, Theme};
use crate::settings::{Choice, Settings, data_dir};
use crate::{actions, ghostty, wallpaper};

const LABEL: &str = "xyz.kanishkk.oms";
const TICK: Duration = Duration::from_secs(5);

fn plist_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"))
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
  <key>Label</key><string>{LABEL}</string>
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
    let entries: String = vars
        .iter()
        .map(|(k, v)| {
            let v = v.replace('&', "&amp;").replace('<', "&lt;");
            format!("    <key>{k}</key><string>{v}</string>\n")
        })
        .collect();
    format!("  <key>EnvironmentVariables</key>\n  <dict>\n{entries}  </dict>\n")
}

fn install() -> Result<()> {
    let path = plist_path();
    let text = plist()?;
    let loaded = Command::new("launchctl")
        .args(["print", &format!("{}/{LABEL}", domain())])
        .output()
        .is_ok_and(|o| o.status.success());
    if loaded && fs::read_to_string(&path).is_ok_and(|t| t == text) {
        return Ok(());
    }
    fs::create_dir_all(path.parent().context("bad LaunchAgents path")?)?;
    fs::create_dir_all(data_dir())?;
    fs::write(&path, text)?;
    let _ = Command::new("launchctl")
        .args(["bootout", &format!("{}/{LABEL}", domain())])
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

pub fn uninstall() -> Result<()> {
    let path = plist_path();
    if path.exists() {
        let _ = Command::new("launchctl")
            .args(["bootout", &format!("{}/{LABEL}", domain())])
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

/// The agent's loop. Runs until launchd stops it.
pub fn run() -> Result<()> {
    let repos = Repos::locate(None, None)?;
    // What the agent last switched to, so a new light/dark pick applies at once.
    let mut last: Option<Choice> = None;
    let mut last_rotation = Instant::now();
    log("started");
    loop {
        let settings = Settings::load();
        let themes = repos.load(&settings).unwrap_or_default();
        let dark = is_dark();

        if settings.auto
            && let (Some(light), Some(dark_choice)) = (&settings.light, &settings.dark)
        {
            let choice = if dark { dark_choice } else { light };
            if last.as_ref() != Some(choice) {
                if let Err(e) = follow_appearance(&themes, choice, &settings) {
                    log(&format!("couldn't switch to {}: {e:#}", choice.theme));
                }
                last = Some(choice.clone());
                last_rotation = Instant::now();
            }
        } else {
            last = None;
        }

        if let Some(minutes) = settings.rotate_minutes
            && last_rotation.elapsed() >= Duration::from_secs(minutes.max(1) * 60)
        {
            if let Err(e) = rotate(&themes, &settings, dark) {
                log(&format!("couldn't rotate: {e:#}"));
            }
            last_rotation = Instant::now();
        }
        sleep(TICK);
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
