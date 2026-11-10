//! Wayland compositors without a wallpaper setting of their own: Hyprland
//! (hyprpaper) and Sway (swaybg through `swaymsg`). If the swww daemon is
//! running, it's used on either, as that's what it's there for.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;

use super::run;

fn runtime_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)
}

/// The newest entry in `dir` whose name `matches`, e.g. the socket of the
/// session that's running now rather than one left from an earlier login.
fn newest(dir: &Path, matches: impl Fn(&str) -> bool) -> Option<PathBuf> {
    fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| matches(&e.file_name().to_string_lossy()))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
}

/// A command that reaches the running compositor even when the agent was
/// started with a previous session's (or no) socket variables.
pub(crate) fn session_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    let Some(runtime) = runtime_dir() else {
        return cmd;
    };
    let hypr = runtime.join("hypr");
    let signature_ok =
        std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some_and(|sig| hypr.join(sig).exists());
    if !signature_ok && let Some(dir) = newest(&hypr, |_| true) {
        cmd.env(
            "HYPRLAND_INSTANCE_SIGNATURE",
            dir.file_name().unwrap_or_default(),
        );
    }
    let sway_ok = std::env::var_os("SWAYSOCK").is_some_and(|s| Path::new(&s).exists());
    if !sway_ok
        && let Some(sock) = newest(&runtime, |n| {
            n.starts_with("sway-ipc.") && Path::new(n).extension().is_some_and(|e| e == "sock")
        })
    {
        cmd.env("SWAYSOCK", sock);
    }
    // mpvpaper and Omarchy's tools talk to the compositor directly.
    let wayland_ok = std::env::var_os("WAYLAND_DISPLAY")
        .is_some_and(|d| runtime.join(&d).exists() || Path::new(&d).exists());
    if !wayland_ok
        && let Some(sock) = newest(&runtime, |n| {
            n.starts_with("wayland-") && Path::new(n).extension().is_none()
        })
    {
        cmd.env("WAYLAND_DISPLAY", sock.file_name().unwrap_or_default());
    }
    cmd
}

/// Whether Hyprland has something to show a picture with: hyprpaper or swww.
pub fn hyprland_has_wallpaper_tool() -> bool {
    swww_running() || super::process_running("hyprpaper")
}

fn swww_running() -> bool {
    session_command("swww")
        .arg("query")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn set_swww(path: &Path) -> Result<()> {
    run(session_command("swww").arg("img").arg(path))?;
    Ok(())
}

/// The image in `swww query` output (`eDP-1: 1920x1080, scale: 1, currently
/// displaying: image: /path`).
fn parse_swww(text: &str) -> Option<PathBuf> {
    let line = text.lines().find(|l| l.contains("image: "))?;
    let path = line.rsplit_once("image: ")?.1.trim();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

fn current_swww() -> Option<PathBuf> {
    parse_swww(&run(session_command("swww").arg("query")).ok()?)
}

pub fn set_hyprland(path: &Path) -> Result<()> {
    if swww_running() {
        return set_swww(path);
    }
    let image = path.to_string_lossy();
    // Newer hyprpaper loads the picture by itself; older needs `preload` first.
    let _ = run(session_command("hyprctl").args(["hyprpaper", "preload", &image]));
    run(session_command("hyprctl").args(["hyprpaper", "wallpaper", &format!(",{image}")]))
        .map_err(|e| e.context("is hyprpaper (or swww) running?"))?;
    let _ = run(session_command("hyprctl").args(["hyprpaper", "unload", "unused"]));
    Ok(())
}

pub fn current_hyprland() -> Option<PathBuf> {
    if swww_running() {
        return current_swww();
    }
    let out = run(session_command("hyprctl").args(["hyprpaper", "listactive"])).ok()?;
    out.lines()
        .find_map(|l| l.split_once(" = "))
        .map(|(_, path)| PathBuf::from(path.trim()))
}

pub fn set_sway(path: &Path) -> Result<()> {
    if swww_running() {
        return set_swww(path);
    }
    run(session_command("swaymsg")
        .args(["output", "*", "bg"])
        .arg(path)
        .arg("fill"))?;
    Ok(())
}

/// swaybg's command line holds the picture (`swaybg -o * -i /path -m fill`).
pub fn current_sway() -> Option<PathBuf> {
    if swww_running() {
        return current_swww();
    }
    let out = run(Command::new("pgrep").args(["-a", "swaybg"])).ok()?;
    parse_swaybg(&out)
}

fn parse_swaybg(text: &str) -> Option<PathBuf> {
    let line = text.lines().next()?;
    let mut words = line.split_whitespace();
    words.find(|w| *w == "-i" || *w == "--image")?;
    words.next().map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_swww_and_swaybg() {
        let swww = ": eDP-1: 1920x1080, scale: 1, currently displaying: image: /home/me/w.png\n";
        assert_eq!(
            parse_swww(swww).as_deref(),
            Some(Path::new("/home/me/w.png"))
        );
        assert_eq!(
            parse_swww("eDP-1: 1920x1080, currently displaying: color: 000000"),
            None
        );
        let swaybg = "1234 swaybg -o * -i /home/me/w.jpg -m fill\n";
        assert_eq!(
            parse_swaybg(swaybg).as_deref(),
            Some(Path::new("/home/me/w.jpg"))
        );
    }
}
