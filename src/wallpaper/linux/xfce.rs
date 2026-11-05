//! XFCE (Kali's default desktop): xfdesktop keeps one backdrop per monitor and
//! workspace in xfconf, and redraws as soon as one changes.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;

use super::run;

const CHANNEL: &str = "xfce4-desktop";
/// xfdesktop's "Zoomed": fill the screen, cropping what doesn't fit.
const ZOOMED: &str = "5";

fn xfconf() -> Command {
    let mut cmd = Command::new("xfconf-query");
    cmd.args(["-c", CHANNEL]);
    cmd
}

/// The `last-image` properties of every monitor and workspace that has one.
fn image_properties(listing: &str) -> Vec<String> {
    let mut props: Vec<String> = listing
        .lines()
        .map(str::trim)
        .filter(|p| p.starts_with("/backdrop/") && p.ends_with("/last-image"))
        .map(str::to_string)
        .collect();
    props.sort();
    props
}

/// Monitor names from `xrandr --listmonitors` (` 0: +*eDP-1 1920/344x1080/194+0+0  eDP-1`).
fn monitor_names(listing: &str) -> Vec<String> {
    listing
        .lines()
        .skip(1)
        .filter_map(|l| l.split_whitespace().last())
        .map(str::to_string)
        .collect()
}

/// Properties to create when xfdesktop hasn't saved any yet (a fresh login).
fn new_properties() -> Vec<String> {
    let monitors = Command::new("xrandr")
        .arg("--listmonitors")
        .output()
        .ok()
        .map(|o| monitor_names(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();
    let monitors = if monitors.is_empty() {
        vec!["0".to_string()]
    } else {
        monitors
    };
    monitors
        .iter()
        .map(|m| format!("/backdrop/screen0/monitor{m}/workspace0/last-image"))
        .collect()
}

pub fn set(path: &Path) -> Result<()> {
    let listing = run(xfconf().arg("-l"))?;
    let mut props = image_properties(&listing);
    if props.is_empty() {
        props = new_properties();
    }
    let image = path.to_string_lossy();
    for prop in &props {
        // -n -t creates the property if it's missing and sets it either way.
        run(xfconf().args(["-p", prop, "-n", "-t", "string", "-s", &image]))?;
        if let Some(dir) = prop.strip_suffix("/last-image") {
            let style = format!("{dir}/image-style");
            let _ = run(xfconf().args(["-p", &style, "-n", "-t", "int", "-s", ZOOMED]));
        }
    }
    Ok(())
}

pub fn current() -> Option<PathBuf> {
    let listing = run(xfconf().arg("-l")).ok()?;
    let prop = image_properties(&listing).into_iter().next()?;
    let value = run(xfconf().args(["-p", &prop])).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| PathBuf::from(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_every_backdrop_image() {
        let listing = "/backdrop/screen0/monitorVNC-0/workspace0/color-style\n\
            /backdrop/screen0/monitorVNC-0/workspace1/last-image\n\
            /backdrop/screen0/monitorVNC-0/workspace0/last-image\n\
            /desktop-icons/style\n";
        assert_eq!(
            image_properties(listing),
            [
                "/backdrop/screen0/monitorVNC-0/workspace0/last-image",
                "/backdrop/screen0/monitorVNC-0/workspace1/last-image",
            ]
        );
    }

    #[test]
    fn reads_monitor_names() {
        let listing = "Monitors: 2\n 0: +*eDP-1 1920/344x1080/194+0+0  eDP-1\n 1: +HDMI-1 2560/597x1440/336+1920+0  HDMI-1\n";
        assert_eq!(monitor_names(listing), ["eDP-1", "HDMI-1"]);
    }
}
