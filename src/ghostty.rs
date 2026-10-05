//! Installing the theme files, editing Ghostty's config and reloading it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map_or_else(
            || dirs::home_dir().unwrap_or_default().join(".config"),
            PathBuf::from,
        )
        .join("ghostty")
}

/// Copies the Omarchy theme files into Ghostty's user theme directory so
/// `theme = Omarchy ...` resolves. Returns how many files changed.
pub fn install_themes(repo: &Path) -> Result<usize> {
    let dest = config_home().join("themes");
    fs::create_dir_all(&dest)?;
    let mut changed = 0;
    for entry in fs::read_dir(repo.join("themes"))? {
        let src = entry?.path();
        let Some(name) = src.file_name() else {
            continue;
        };
        if !name.to_string_lossy().starts_with("Omarchy ") {
            continue;
        }
        let new = fs::read(&src)?;
        let target = dest.join(name);
        if fs::read(&target).ok().as_ref() != Some(&new) {
            fs::write(&target, new).with_context(|| format!("writing {}", target.display()))?;
            changed += 1;
        }
    }
    Ok(changed)
}

/// Removes the Omarchy theme files that `install_themes` added.
pub fn remove_themes() -> Result<usize> {
    let mut removed = 0;
    if let Ok(entries) = fs::read_dir(config_home().join("themes")) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with("Omarchy ") {
                fs::remove_file(entry.path())?;
                removed += 1;
            }
        }
    }
    Ok(removed)
}

/// The config file that decides the theme: the last one Ghostty loads that sets
/// `theme`, else the first existing one, else a new `~/.config/ghostty/config`.
pub fn config_path() -> PathBuf {
    let app_support = dirs::data_dir()
        .unwrap_or_default()
        .join("com.mitchellh.ghostty");
    // Load order on macOS; later files override earlier ones.
    let candidates = [
        config_home().join("config.ghostty"),
        config_home().join("config"),
        app_support.join("config.ghostty"),
        app_support.join("config"),
    ];
    candidates
        .iter()
        .rev()
        .find(|p| fs::read_to_string(p).is_ok_and(|t| theme_line(&t).is_some()))
        .or_else(|| candidates.iter().find(|p| p.exists()))
        .unwrap_or(&candidates[1])
        .clone()
}

/// Index and value of the last active `theme = ...` line.
fn theme_line(text: &str) -> Option<(usize, &str)> {
    text.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let (key, value) = line.split_once('=')?;
            (!line.trim_start().starts_with('#') && key.trim() == "theme")
                .then(|| (i, value.trim()))
        })
        .last()
}

pub fn current_theme(config: &Path) -> Option<String> {
    let text = fs::read_to_string(config).ok()?;
    theme_line(&text).map(|(_, v)| v.to_string())
}

/// Returns the config text with its theme set to `theme`.
pub fn with_theme(text: &str, theme: &str) -> String {
    let line = format!("theme = {theme}");
    match theme_line(text) {
        Some((index, _)) => {
            let mut lines: Vec<&str> = text.lines().collect();
            lines[index] = &line;
            let mut out = lines.join("\n");
            if text.ends_with('\n') {
                out.push('\n');
            }
            out
        }
        None if text.is_empty() || text.ends_with('\n') => format!("{text}{line}\n"),
        None => format!("{text}\n{line}\n"),
    }
}

pub fn read_config(config: &Path) -> Option<String> {
    fs::read_to_string(config).ok()
}

/// Writes the config (or removes it, for `None`) and tells Ghostty to reload.
pub fn write_config(config: &Path, text: Option<&str>) -> Result<()> {
    match text {
        Some(text) => {
            if let Some(dir) = config.parent() {
                fs::create_dir_all(dir)?;
            }
            // Written in place so a symlinked config (dotfiles) stays a symlink.
            fs::write(config, text).with_context(|| format!("writing {}", config.display()))?;
        }
        None => {
            let _ = fs::remove_file(config);
        }
    }
    reload();
    Ok(())
}

pub fn set_theme(config: &Path, theme: &str) -> Result<()> {
    let text = read_config(config).unwrap_or_default();
    write_config(config, Some(&with_theme(&text, theme)))
}

/// Ghostty reloads its config on SIGUSR2, recoloring every open window.
pub fn reload() {
    // -a: macOS pkill skips its own ancestors by default, which would leave out
    // the Ghostty this is running in.
    let _ = Command::new("pkill")
        .args(["-USR2", "-a", "-x", "ghostty"])
        .output();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_last_active_theme_line() {
        let text = "font-size = 14\ntheme = A\n# theme = B\ntheme = C\nx = 1\n";
        assert_eq!(
            with_theme(text, "D"),
            "font-size = 14\ntheme = A\n# theme = B\ntheme = D\nx = 1\n"
        );
    }

    #[test]
    fn appends_when_missing() {
        assert_eq!(with_theme("", "D"), "theme = D\n");
        assert_eq!(with_theme("a = 1", "D"), "a = 1\ntheme = D\n");
        assert_eq!(with_theme("# theme = X\n", "D"), "# theme = X\ntheme = D\n");
    }

    #[test]
    fn keeps_light_dark_value() {
        let text = "theme = light:A,dark:B\n";
        assert_eq!(theme_line(text), Some((0, "light:A,dark:B")));
    }
}
