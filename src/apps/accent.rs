//! The system accent color, closest to the theme's accent: macOS's
//! `AppleAccentColor`, or GNOME 47+'s `accent-color` on Linux.

use anyhow::Result;
#[cfg(target_os = "macos")]
use anyhow::bail;
use std::process::Command;

use crate::repo::Rgb;

/// What the app list says the accent does.
#[cfg(target_os = "macos")]
pub const DESCRIPTION: &str = "macOS accent color, closest to the theme's accent";
#[cfg(not(target_os = "macos"))]
pub const DESCRIPTION: &str = "GNOME accent color (GNOME 47+), closest to the theme's accent";

/// The color's hue in degrees, or `None` for greys (too little saturation).
fn hue((r, g, b): Rgb) -> Option<f32> {
    let (r, g, b) = (
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let lightness = f32::midpoint(max, min);
    let saturation = if delta == 0.0 {
        0.0
    } else {
        delta / (1.0 - (2.0 * lightness - 1.0).abs())
    };
    if saturation < 0.18 || delta < 0.08 {
        return None;
    }
    // `max` is exactly one of r, g, b, so these comparisons are exact.
    #[allow(clippy::float_cmp)]
    let hue = if max == r {
        60.0 * (((g - b) / delta).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    Some(hue)
}

/// The entry in `accents` (value, hue in degrees) closest to `hue`.
fn closest<T: Copy>(accents: &[(T, f32)], hue: f32, fallback: T) -> T {
    accents
        .iter()
        .min_by(|a, b| (a.1 - hue).abs().total_cmp(&(b.1 - hue).abs()))
        .map_or(fallback, |a| a.0)
}

/// macOS's accent colors: (`AppleAccentColor` value, hue in degrees).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const MAC_ACCENTS: &[(i32, f32)] = &[
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
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn nearest_accent(color: Rgb) -> i32 {
    hue(color).map_or(-1, |h| closest(MAC_ACCENTS, h, 4))
}

/// GNOME's accent colors (hue in degrees).
#[cfg_attr(target_os = "macos", allow(dead_code))]
const GNOME_ACCENTS: &[(&str, f32)] = &[
    ("red", 0.0),
    ("orange", 30.0),
    ("yellow", 55.0),
    ("green", 120.0),
    ("teal", 180.0),
    ("blue", 215.0),
    ("purple", 275.0),
    ("pink", 330.0),
    ("red", 360.0),
];

/// The GNOME accent closest to `color`; slate for greys.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn nearest_gnome_accent(color: Rgb) -> &'static str {
    hue(color).map_or("slate", |h| closest(GNOME_ACCENTS, h, "blue"))
}

#[cfg(target_os = "macos")]
pub fn available() -> bool {
    true
}

/// GNOME 47+ has the setting; other desktops don't, so there's nothing to set.
#[cfg(not(target_os = "macos"))]
pub fn available() -> bool {
    Command::new("gsettings")
        .args(["range", "org.gnome.desktop.interface", "accent-color"])
        .output()
        .is_ok_and(|o| o.status.success())
}

#[cfg(target_os = "macos")]
pub fn apply(color: Rgb) -> Result<()> {
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
    notify_changed();
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn apply(color: Rgb) -> Result<()> {
    if !available() {
        anyhow::bail!("this desktop has no accent color setting (it needs GNOME 47 or newer)");
    }
    let out = Command::new("gsettings")
        .args([
            "set",
            "org.gnome.desktop.interface",
            "accent-color",
            nearest_gnome_accent(color),
        ])
        .output()?;
    if !out.status.success() {
        anyhow::bail!(
            "gsettings failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn remove() {
    let _ = Command::new("defaults")
        .args(["delete", "-g", "AppleAccentColor"])
        .output();
    notify_changed();
}

#[cfg(not(target_os = "macos"))]
pub fn remove() {
    let _ = Command::new("gsettings")
        .args(["reset", "org.gnome.desktop.interface", "accent-color"])
        .output();
}

/// Tells running apps the accent changed, so most update without a restart.
#[cfg(target_os = "macos")]
fn notify_changed() {
    use objc2_foundation::{NSDistributedNotificationCenter, NSString};
    let center = NSDistributedNotificationCenter::defaultCenter();
    for name in [
        "AppleColorPreferencesChangedNotification",
        "AppleAquaColorVariantChanged",
    ] {
        // SAFETY: a valid notification name with no object or user info, which
        // the API allows; posting a distributed notification has no other preconditions.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_colors_to_macos_accents() {
        assert_eq!(nearest_accent((0x7a, 0xa2, 0xf7)), 4); // Tokyo Night blue
        assert_eq!(nearest_accent((0xf7, 0x76, 0x8e)), 0); // red
        assert_eq!(nearest_accent((0x9e, 0xce, 0x6a)), 3); // green
        assert_eq!(nearest_accent((0xad, 0x8e, 0xe6)), 5); // purple
        assert_eq!(nearest_accent((0xe0, 0xaf, 0x68)), 1); // amber -> orange
        assert_eq!(nearest_accent((0x88, 0x88, 0x88)), -1); // grey -> graphite
    }

    #[test]
    fn maps_colors_to_gnome_accents() {
        assert_eq!(nearest_gnome_accent((0x7a, 0xa2, 0xf7)), "blue");
        assert_eq!(nearest_gnome_accent((0xf7, 0x76, 0x8e)), "red");
        assert_eq!(nearest_gnome_accent((0x9e, 0xce, 0x6a)), "green");
        assert_eq!(nearest_gnome_accent((0x7d, 0xcf, 0xcf)), "teal");
        assert_eq!(nearest_gnome_accent((0x88, 0x88, 0x88)), "slate");
    }
}
