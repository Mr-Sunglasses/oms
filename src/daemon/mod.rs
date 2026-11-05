//! The background agent (`oms daemon`, run by launchd on macOS and by a
//! systemd user service on Linux). It switches the wallpaper and app themes
//! when the system changes between light and dark, and rotates wallpapers.
//! Ghostty switches its own theme (`theme = light:…,dark:…`).

#[cfg(target_os = "macos")]
mod launchd;
#[cfg(target_os = "linux")]
mod linux_appearance;
#[cfg(target_os = "linux")]
mod systemd;

#[cfg(target_os = "macos")]
use launchd as platform;
#[cfg(target_os = "linux")]
use systemd as platform;

pub use platform::{
    is_dark, is_installed, is_running, other_agents, post_settings_changed, remove_hint,
    remove_other, restart_if_running, uninstall,
};

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::repo::{Repos, Theme};
use crate::settings::{Choice, Settings};
use crate::{actions, ghostty, wallpaper};

const LABEL: &str = "xyz.kanishkk.oms";

/// The agent's name (launchd label, or systemd unit name without `.service`).
/// A custom `OMS_DATA_DIR` gets its own agent, so a second setup (or a test)
/// never replaces or removes the main one.
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

/// A check now and then even without notifications, in case one was missed.
const SAFETY_TICK: Duration = Duration::from_secs(300);

/// An oms agent other than this setup's, e.g. one left behind by a run with a
/// different `OMS_DATA_DIR`. It still switches the real wallpaper.
pub struct OtherAgent {
    pub label: String,
    /// Its launchd plist or systemd unit file.
    pub file: PathBuf,
    /// The binary it runs, if the file could be read.
    pub program: Option<PathBuf>,
}

/// The label in an agent file name (`<label><extension>`), if it's an oms
/// agent other than `own`.
fn other_label(file_name: &str, extension: &str, own: &str) -> Option<String> {
    let label = file_name.strip_suffix(extension)?;
    let is_oms = label == LABEL || label.strip_prefix(LABEL)?.starts_with('.');
    (is_oms && label != own).then(|| label.to_string())
}

/// Installs or removes the agent to match the settings.
pub fn sync(settings: &Settings) -> Result<()> {
    if settings.needs_daemon() {
        platform::install()
    } else {
        uninstall()
    }
}

fn log(message: &str) {
    eprintln!("{} {message}", crate::clock::log_stamp());
}

/// The agent's loop. Runs until the service manager stops it. It sleeps until
/// the system changes appearance, oms changes its settings, or a wallpaper is due.
pub fn run() -> Result<()> {
    let repos = Repos::locate(None, None)?;
    let woken = Arc::new(AtomicBool::new(true));
    let mut watcher = platform::Watcher::new(&woken);
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
        watcher.wait(
            timeout.max(Duration::from_millis(100)),
            &woken,
            settings.auto,
        );
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
    // Linux: Ghostty gets the mode's theme from oms (see `light_dark_value`).
    #[cfg(target_os = "linux")]
    {
        let config = ghostty::config_path();
        if ghostty::current_theme(&config).as_deref() != Some(theme.ghostty_name.as_str()) {
            ghostty::set_theme(&config, &theme.ghostty_name)?;
        }
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

#[cfg(test)]
mod tests {
    use super::other_label;

    #[test]
    fn finds_other_oms_agents_only() {
        let own = "xyz.kanishkk.oms";
        for ext in [".plist", ".service"] {
            let name = |label: &str| format!("{label}{ext}");
            assert_eq!(
                other_label(&name("xyz.kanishkk.oms.b8919715"), ext, own).as_deref(),
                Some("xyz.kanishkk.oms.b8919715")
            );
            assert_eq!(other_label(&name("xyz.kanishkk.oms"), ext, own), None);
            assert_eq!(
                other_label(&name("xyz.kanishkk.oms"), ext, "xyz.kanishkk.oms.1234abcd").as_deref(),
                Some("xyz.kanishkk.oms")
            );
            assert_eq!(other_label(&name("xyz.kanishkk.omsx"), ext, own), None);
            assert_eq!(other_label(&name("com.apple.foo"), ext, own), None);
            assert_eq!(other_label("xyz.kanishkk.oms.b8919715", ext, own), None);
        }
    }
}
