//! Theming apps beyond Ghostty: Neovim, btop, bat, tmux and the macOS accent
//! color. Each is opt-in (`oms apps on <app>`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::repo::{Rgb, Theme};

pub const APPS: &[(&str, &str)] = &[
    (
        "nvim",
        "Neovim (LazyVim): colorscheme plugin in lua/plugins/omarchy-theme.lua",
    ),
    ("btop", "btop: color theme \"omarchy\""),
    ("bat", "bat: syntax theme \"omarchy\" (also used by delta)"),
    ("tmux", "tmux: status bar, borders and messages"),
    (
        "accent",
        "macOS accent color, closest to the theme's accent",
    ),
];

pub fn is_known(app: &str) -> bool {
    APPS.iter().any(|(id, _)| *id == app)
}

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".config"))
}

fn on_path(program: &str) -> bool {
    Command::new("/usr/bin/which")
        .arg(program)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Whether the app looks installed, so it's worth theming.
pub fn available(app: &str) -> bool {
    match app {
        "nvim" => config_home().join("nvim").exists(),
        "btop" | "bat" | "tmux" => on_path(app),
        "accent" => true,
        _ => false,
    }
}

/// Applies `theme` to each enabled app. Returns the ones that worked, and
/// failures as "app: reason".
pub fn apply(theme: &Theme, enabled: &[String]) -> (Vec<String>, Vec<String>) {
    let (mut done, mut failed) = (Vec::new(), Vec::new());
    for app in enabled {
        let result = match app.as_str() {
            "nvim" => nvim(theme),
            "btop" => btop(theme),
            "bat" => bat(theme),
            "tmux" => tmux(theme),
            "accent" => accent(theme.accent),
            _ => continue,
        };
        match result {
            Ok(()) => done.push(app.clone()),
            Err(e) => failed.push(format!("{app}: {e:#}")),
        }
    }
    (done, failed)
}

fn read_app_file(theme: &Theme, name: &str) -> Result<String> {
    let path = theme.apps_dir.join(name);
    fs::read_to_string(&path)
        .with_context(|| format!("{} is missing (run `oms update`)", path.display()))
}

/// Writes `text` to `path` (creating folders) unless it's already there.
fn write(path: &Path, text: &str) -> Result<()> {
    if fs::read_to_string(path).is_ok_and(|t| t == text) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

/// Sets `key` in a simple `key = value` / `key value` file: replaces the first
/// active line that `is_key` matches, or appends `line`.
fn set_line(path: &Path, is_key: impl Fn(&str) -> bool, line: &str) -> Result<()> {
    let text = fs::read_to_string(path).unwrap_or_default();
    let mut found = false;
    let mut lines: Vec<String> = text
        .lines()
        .map(|l| {
            if !found && !l.trim_start().starts_with('#') && is_key(l.trim_start()) {
                found = true;
                line.to_string()
            } else {
                l.to_string()
            }
        })
        .collect();
    if !found {
        lines.push(line.to_string());
    }
    write(path, &(lines.join("\n") + "\n"))
}

/// Removes lines that `is_key` matches.
fn remove_lines(path: &Path, is_key: impl Fn(&str) -> bool) -> Result<()> {
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(());
    };
    let kept: Vec<&str> = text.lines().filter(|l| !is_key(l.trim_start())).collect();
    write(path, &(kept.join("\n") + "\n"))
}

fn nvim_file() -> PathBuf {
    config_home().join("nvim/lua/plugins/omarchy-theme.lua")
}

fn nvim(theme: &Theme) -> Result<()> {
    if !config_home().join("nvim/lua/plugins").exists() {
        bail!("needs a LazyVim-style config (~/.config/nvim/lua/plugins)");
    }
    write(&nvim_file(), &read_app_file(theme, "neovim.lua")?)
}

fn btop_dir() -> PathBuf {
    config_home().join("btop")
}

fn btop(theme: &Theme) -> Result<()> {
    write(
        &btop_dir().join("themes/omarchy.theme"),
        &read_app_file(theme, "btop.theme")?,
    )?;
    set_line(
        &btop_dir().join("btop.conf"),
        |l| l.starts_with("color_theme"),
        "color_theme = \"omarchy\"",
    )
}

fn bat_dirs() -> Result<(PathBuf, PathBuf)> {
    let out = Command::new("bat")
        .arg("--config-dir")
        .output()
        .context("running bat")?;
    let dir = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let out = Command::new("bat").arg("--config-file").output()?;
    let file = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    Ok((dir, file))
}

fn bat(theme: &Theme) -> Result<()> {
    let (dir, config) = bat_dirs()?;
    let theme_file = dir.join("themes/omarchy.tmTheme");
    let text = read_app_file(theme, "bat.tmTheme")?;
    let changed = fs::read_to_string(&theme_file).ok().as_deref() != Some(text.as_str());
    write(&theme_file, &text)?;
    if changed {
        let out = Command::new("bat").args(["cache", "--build"]).output()?;
        if !out.status.success() {
            bail!("bat cache --build failed");
        }
    }
    set_line(&config, |l| l.starts_with("--theme"), "--theme=\"omarchy\"")
}

fn tmux_theme_file() -> PathBuf {
    config_home().join("tmux/omarchy-theme.conf")
}

fn tmux_conf() -> PathBuf {
    let xdg = config_home().join("tmux/tmux.conf");
    let home = dirs::home_dir().unwrap_or_default().join(".tmux.conf");
    if xdg.exists() || !home.exists() {
        xdg
    } else {
        home
    }
}

fn tmux_source_line() -> String {
    format!("source-file {}", tmux_theme_file().display())
}

fn tmux(theme: &Theme) -> Result<()> {
    let file = tmux_theme_file();
    write(&file, &read_app_file(theme, "tmux.conf")?)?;
    let conf = tmux_conf();
    let line = tmux_source_line();
    if !fs::read_to_string(&conf)
        .unwrap_or_default()
        .lines()
        .any(|l| l.trim() == line)
    {
        let mut text = fs::read_to_string(&conf).unwrap_or_default();
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&format!("# Colors from oms (Omarchy themes)\n{line}\n"));
        write(&conf, &text)?;
    }
    // Recolor running tmux sessions right away; fine if none are running.
    let _ = Command::new("tmux").arg("source-file").arg(&file).output();
    Ok(())
}

/// macOS's accent colors: (AppleAccentColor value, hue in degrees).
const ACCENTS: &[(i32, f32)] = &[
    (0, 0.0),   // red
    (1, 30.0),  // orange
    (2, 55.0),  // yellow
    (3, 120.0), // green
    (4, 215.0), // blue
    (5, 275.0), // purple
    (6, 330.0), // pink
    (0, 360.0), // red again, for hues near 360
];

/// The macOS accent closest to `color`; -1 (graphite) for greys.
pub fn nearest_accent((r, g, b): Rgb) -> i32 {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let lightness = (max + min) / 2.0;
    let saturation = if delta == 0.0 {
        0.0
    } else {
        delta / (1.0 - (2.0 * lightness - 1.0).abs())
    };
    if saturation < 0.18 || delta < 0.08 {
        return -1;
    }
    let hue = if max == r {
        60.0 * (((g - b) / delta).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    ACCENTS
        .iter()
        .min_by(|a, b| (a.1 - hue).abs().total_cmp(&(b.1 - hue).abs()))
        .map(|a| a.0)
        .unwrap_or(4)
}

fn accent(color: Rgb) -> Result<()> {
    let value = nearest_accent(color);
    let status = Command::new("defaults")
        .args([
            "write",
            "-g",
            "AppleAccentColor",
            "-int",
            &value.to_string(),
        ])
        .status()?;
    if !status.success() {
        bail!("defaults write failed");
    }
    // Leave the highlight color on "Automatic" so it follows the accent.
    let _ = Command::new("defaults")
        .args(["delete", "-g", "AppleHighlightColor"])
        .output();
    notify_accent_changed();
    Ok(())
}

/// Tells running apps the accent changed, so most update without a restart.
fn notify_accent_changed() {
    use objc2_foundation::{NSDistributedNotificationCenter, NSString};
    let center = NSDistributedNotificationCenter::defaultCenter();
    for name in [
        "AppleColorPreferencesChangedNotification",
        "AppleAquaColorVariantChanged",
    ] {
        unsafe {
            center.postNotificationName_object_userInfo_deliverImmediately(
                &NSString::from_str(name),
                None,
                None,
                true,
            );
        }
    }
}

/// Undoes what `apply` set up for `app`.
pub fn remove(app: &str) -> Result<()> {
    match app {
        "nvim" => {
            let _ = fs::remove_file(nvim_file());
        }
        "btop" => {
            let _ = fs::remove_file(btop_dir().join("themes/omarchy.theme"));
            remove_lines(&btop_dir().join("btop.conf"), |l| {
                l.starts_with("color_theme") && l.contains("omarchy")
            })?;
        }
        "bat" => {
            if let Ok((dir, config)) = bat_dirs() {
                let _ = fs::remove_file(dir.join("themes/omarchy.tmTheme"));
                remove_lines(&config, |l| {
                    l.starts_with("--theme") && l.contains("omarchy")
                })?;
                let _ = Command::new("bat").args(["cache", "--build"]).output();
            }
        }
        "tmux" => {
            let _ = fs::remove_file(tmux_theme_file());
            let line = tmux_source_line();
            remove_lines(&tmux_conf(), |l| {
                l.trim() == line || l.trim() == "# Colors from oms (Omarchy themes)"
            })?;
        }
        "accent" => {
            let _ = Command::new("defaults")
                .args(["delete", "-g", "AppleAccentColor"])
                .output();
            notify_accent_changed();
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::nearest_accent;

    #[test]
    fn maps_colors_to_macos_accents() {
        assert_eq!(nearest_accent((0x7a, 0xa2, 0xf7)), 4); // Tokyo Night blue
        assert_eq!(nearest_accent((0xf7, 0x76, 0x8e)), 0); // red
        assert_eq!(nearest_accent((0x9e, 0xce, 0x6a)), 3); // green
        assert_eq!(nearest_accent((0xad, 0x8e, 0xe6)), 5); // purple
        assert_eq!(nearest_accent((0xe0, 0xaf, 0x68)), 1); // amber -> orange
        assert_eq!(nearest_accent((0x88, 0x88, 0x88)), -1); // grey -> graphite
    }
}
