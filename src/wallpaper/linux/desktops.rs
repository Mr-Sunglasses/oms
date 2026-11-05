//! GNOME, KDE Plasma and plain X11 window managers (feh).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;

use super::run;
use crate::wallpaper::{file_url, url_path};

const GNOME_BACKGROUND: &str = "org.gnome.desktop.background";

/// GNOME keeps a separate picture for dark mode; set both so the wallpaper
/// doesn't jump back when the mode changes.
pub fn set_gnome(path: &Path) -> Result<()> {
    let url = file_url(path);
    run(Command::new("gsettings").args(["set", GNOME_BACKGROUND, "picture-uri", &url]))?;
    // Missing before GNOME 42; fine to skip there.
    let _ =
        run(Command::new("gsettings").args(["set", GNOME_BACKGROUND, "picture-uri-dark", &url]));
    let _ =
        run(Command::new("gsettings").args(["set", GNOME_BACKGROUND, "picture-options", "zoom"]));
    Ok(())
}

pub fn current_gnome() -> Option<PathBuf> {
    let out = run(Command::new("gsettings").args(["get", GNOME_BACKGROUND, "picture-uri"])).ok()?;
    url_path(out.trim().trim_matches('\''))
}

pub fn set_kde(path: &Path) -> Result<()> {
    run(Command::new("plasma-apply-wallpaperimage").arg(path))?;
    Ok(())
}

/// The first `Image=` in Plasma's desktop config (one per monitor).
pub fn current_kde() -> Option<PathBuf> {
    let config = dirs::config_dir()?.join("plasma-org.kde.plasma.desktop-appletsrc");
    let text = fs::read_to_string(config).ok()?;
    parse_kde(&text)
}

fn parse_kde(text: &str) -> Option<PathBuf> {
    let value = text.lines().find_map(|l| l.trim().strip_prefix("Image="))?;
    url_path(value).or_else(|| Some(PathBuf::from(value)))
}

pub fn set_feh(path: &Path) -> Result<()> {
    run(Command::new("feh").arg("--bg-fill").arg(path))?;
    Ok(())
}

/// feh records the command it ran in `~/.fehbg`.
pub fn current_feh() -> Option<PathBuf> {
    let text = fs::read_to_string(dirs::home_dir()?.join(".fehbg")).ok()?;
    parse_fehbg(&text)
}

fn parse_fehbg(text: &str) -> Option<PathBuf> {
    let line = text.lines().rev().find(|l| l.contains("feh "))?;
    let quoted = line.rsplit('\'').nth(1)?;
    Some(PathBuf::from(quoted))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_kde_and_feh_configs() {
        let plasma = "[Containments][1][Wallpaper][org.kde.image][General]\nImage=file:///home/me/a%20b.png\n";
        assert_eq!(
            parse_kde(plasma).as_deref(),
            Some(Path::new("/home/me/a b.png"))
        );
        let fehbg = "#!/bin/sh\nfeh --no-fehbg --bg-fill '/home/me/wall.jpg' \n";
        assert_eq!(
            parse_fehbg(fehbg).as_deref(),
            Some(Path::new("/home/me/wall.jpg"))
        );
    }
}
