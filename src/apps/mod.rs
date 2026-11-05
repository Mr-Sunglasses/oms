//! Theming apps beyond Ghostty: Neovim, btop, bat, tmux and the system accent
//! color. Each is opt-in (`oms apps on <app>`).

mod accent;
mod nvim;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::repo::Theme;

pub const APPS: &[(&str, &str)] = &[
    (
        "nvim",
        "Neovim (LazyVim): colorscheme plugin in lua/plugins/omarchy-theme.lua",
    ),
    ("btop", "btop: color theme \"omarchy\""),
    ("bat", "bat: syntax theme \"omarchy\" (also used by delta)"),
    ("tmux", "tmux: status bar, borders and messages"),
    ("accent", accent::DESCRIPTION),
];

/// The app ids, for help text and shell completion.
pub fn names() -> Vec<&'static str> {
    APPS.iter().map(|(id, _)| *id).collect()
}

pub fn is_known(app: &str) -> bool {
    APPS.iter().any(|(id, _)| *id == app)
}

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map_or_else(
        || dirs::home_dir().unwrap_or_default().join(".config"),
        PathBuf::from,
    )
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
        "accent" => accent::available(),
        _ => false,
    }
}

/// Applies `theme` to each enabled app. Returns the ones that worked, and
/// failures as "app: reason".
pub fn apply(theme: &Theme, enabled: &[String]) -> (Vec<String>, Vec<String>) {
    let (mut done, mut failed) = (Vec::new(), Vec::new());
    for app in enabled {
        let result = match app.as_str() {
            "nvim" => nvim::apply(theme),
            "btop" => btop(theme),
            "bat" => bat(theme),
            "tmux" => tmux(theme),
            "accent" => accent::apply(theme.accent),
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
        text.push_str("# Colors from oms (Omarchy themes)\n");
        text.push_str(&line);
        text.push('\n');
        write(&conf, &text)?;
    }
    // Recolor running tmux sessions right away; fine if none are running.
    let _ = Command::new("tmux").arg("source-file").arg(&file).output();
    Ok(())
}

/// Undoes what `apply` set up for `app`.
pub fn remove(app: &str) -> Result<()> {
    match app {
        "nvim" => {
            let _ = fs::remove_file(nvim::theme_file());
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
        "accent" => accent::remove(),
        _ => {}
    }
    Ok(())
}
