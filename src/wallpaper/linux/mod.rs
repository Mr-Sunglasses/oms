//! Linux: sets the wallpaper with the desktop's own tool. Which desktop is
//! running comes from `XDG_CURRENT_DESKTOP` (or the session's sockets and
//! processes); `OMS_WALLPAPER_BACKEND` overrides it.

mod desktops;
mod wayland;
mod xfce;

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Overrides the detected desktop: xfce, gnome, kde, hyprland, sway or feh.
pub const BACKEND_ENV: &str = "OMS_WALLPAPER_BACKEND";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Xfce,
    Gnome,
    Kde,
    Hyprland,
    Sway,
    /// Any other X11 window manager.
    Feh,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Backend::Xfce => "XFCE",
            Backend::Gnome => "GNOME",
            Backend::Kde => "KDE Plasma",
            Backend::Hyprland => "Hyprland",
            Backend::Sway => "Sway",
            Backend::Feh => "X11 (feh)",
        }
    }

    /// The program the backend runs, to check it's installed.
    pub fn tool(self) -> &'static str {
        match self {
            Backend::Xfce => "xfconf-query",
            Backend::Gnome => "gsettings",
            Backend::Kde => "plasma-apply-wallpaperimage",
            Backend::Hyprland => "hyprctl",
            Backend::Sway => "swaymsg",
            Backend::Feh => "feh",
        }
    }
}

/// A backend named in `OMS_WALLPAPER_BACKEND` or `XDG_CURRENT_DESKTOP`-style
/// lists (`ubuntu:GNOME`, `KDE`, `XFCE`, `Hyprland`).
fn parse_desktop(value: &str) -> Option<Backend> {
    value
        .split([':', ';'])
        .map(|part| part.trim().to_lowercase())
        .find_map(|part| match part.as_str() {
            "xfce" | "xfce4" => Some(Backend::Xfce),
            "gnome" | "gnome-classic" | "gnome-xorg" | "ubuntu" | "pop" | "budgie"
            | "budgie-desktop" | "unity" => Some(Backend::Gnome),
            "kde" | "plasma" | "kde-plasma" => Some(Backend::Kde),
            "hyprland" => Some(Backend::Hyprland),
            "sway" => Some(Backend::Sway),
            "feh" | "x11" => Some(Backend::Feh),
            _ => None,
        })
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn process_running(name: &str) -> bool {
    Command::new("pgrep")
        .args(["-x", "-u", &crate::clock::uid().to_string(), name])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Which desktop to set the wallpaper on.
pub fn detect() -> Result<Backend> {
    if let Some(value) = env(BACKEND_ENV) {
        return parse_desktop(&value).with_context(|| {
            format!("{BACKEND_ENV}={value} isn't one of xfce, gnome, kde, hyprland, sway or feh")
        });
    }
    let named = [
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_DESKTOP",
        "DESKTOP_SESSION",
    ]
    .iter()
    .filter_map(|k| env(k))
    .find_map(|v| parse_desktop(&v));
    if let Some(backend) = named {
        return Ok(backend);
    }
    let by_process = [
        ("Hyprland", Backend::Hyprland),
        ("sway", Backend::Sway),
        ("xfdesktop", Backend::Xfce),
        ("plasmashell", Backend::Kde),
        ("gnome-shell", Backend::Gnome),
    ];
    if let Some((_, backend)) = by_process.iter().find(|(p, _)| process_running(p)) {
        return Ok(*backend);
    }
    if env("DISPLAY").is_some() && on_path("feh") {
        return Ok(Backend::Feh);
    }
    bail!(
        "couldn't tell which desktop you're using; set {BACKEND_ENV} to xfce, gnome, kde, hyprland, sway or feh"
    )
}

pub fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// Runs a desktop tool and returns its output, with an error that says what
/// to install when it's missing.
fn run(cmd: &mut Command) -> Result<String> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let out = cmd.output().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            anyhow::anyhow!("{program} isn't installed (oms uses it to set the wallpaper)")
        }
        _ => anyhow::anyhow!("running {program}: {e}"),
    })?;
    if !out.status.success() {
        bail!(
            "{program} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Sets the wallpaper on every monitor (and every workspace, where the desktop
/// has one per workspace). Downloads the picture first if only its preview is here.
pub fn set(path: &Path) -> Result<()> {
    crate::repo::ensure_local(path)?;
    let path = path
        .canonicalize()
        .with_context(|| format!("{} not found", path.display()))?;
    match detect()? {
        Backend::Xfce => xfce::set(&path),
        Backend::Gnome => desktops::set_gnome(&path),
        Backend::Kde => desktops::set_kde(&path),
        Backend::Hyprland => wayland::set_hyprland(&path),
        Backend::Sway => wayland::set_sway(&path),
        Backend::Feh => desktops::set_feh(&path),
    }
}

/// On Linux every desktop is set at once, from any thread.
pub fn set_everywhere(path: &Path) -> Result<()> {
    set(path)
}

/// The same as `set` on Linux (macOS needs a main-thread fallback).
pub fn set_current_space(path: &Path) -> Result<()> {
    set(path)
}

/// The current wallpaper, if the desktop says.
pub fn current() -> Option<PathBuf> {
    match detect().ok()? {
        Backend::Xfce => xfce::current(),
        Backend::Gnome => desktops::current_gnome(),
        Backend::Kde => desktops::current_kde(),
        Backend::Hyprland => wayland::current_hyprland(),
        Backend::Sway => wayland::current_sway(),
        Backend::Feh => desktops::current_feh(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_desktop_names() {
        assert_eq!(parse_desktop("XFCE"), Some(Backend::Xfce));
        assert_eq!(parse_desktop("ubuntu:GNOME"), Some(Backend::Gnome));
        assert_eq!(parse_desktop("KDE"), Some(Backend::Kde));
        assert_eq!(parse_desktop("Hyprland"), Some(Backend::Hyprland));
        assert_eq!(parse_desktop("sway"), Some(Backend::Sway));
        assert_eq!(parse_desktop("X-Cinnamon"), None);
        assert_eq!(parse_desktop(""), None);
    }
}
