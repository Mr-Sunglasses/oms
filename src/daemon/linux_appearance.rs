//! Linux: whether the desktop is in dark mode. Desktops store this in
//! different places, so ask the freedesktop portal first (GNOME, KDE and most
//! Wayland setups answer it), then each desktop's own setting.

use std::process::Command;

/// What one source says about the mode.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Mode {
    Dark,
    Light,
    /// The source has no preference (or couldn't be read): ask the next one.
    Unknown,
}

fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn is_dark() -> bool {
    let sources: [fn() -> Mode; 4] = [portal, gnome, xfce, kde];
    sources
        .iter()
        .map(|source| source())
        .find(|mode| *mode != Mode::Unknown)
        == Some(Mode::Dark)
}

/// `org.freedesktop.appearance color-scheme`: 1 is dark, 2 light, 0 no preference.
fn portal() -> Mode {
    let base = [
        "call",
        "--session",
        "--timeout",
        "2",
        "--dest",
        "org.freedesktop.portal.Desktop",
        "--object-path",
        "/org/freedesktop/portal/desktop",
        "--method",
    ];
    [
        "org.freedesktop.portal.Settings.ReadOne",
        "org.freedesktop.portal.Settings.Read",
    ]
    .iter()
    .find_map(|method| {
        let mut args = base.to_vec();
        args.extend([*method, "org.freedesktop.appearance", "color-scheme"]);
        output("gdbus", &args)
    })
    .map_or(Mode::Unknown, |text| parse_portal(&text))
}

/// Reads `(<uint32 1>,)` (or `(<<uint32 1>>,)` from the older `Read`).
fn parse_portal(text: &str) -> Mode {
    let value = text
        .split("uint32")
        .nth(1)
        .and_then(|rest| {
            rest.trim_start()
                .split(|c: char| !c.is_ascii_digit())
                .next()
        })
        .and_then(|n| n.parse::<u32>().ok());
    match value {
        Some(1) => Mode::Dark,
        Some(2) => Mode::Light,
        _ => Mode::Unknown,
    }
}

/// GNOME 42+: `color-scheme` is 'prefer-dark', 'prefer-light' or 'default'.
fn gnome() -> Mode {
    output(
        "gsettings",
        &["get", "org.gnome.desktop.interface", "color-scheme"],
    )
    .map_or(Mode::Unknown, |text| parse_gnome(&text))
}

fn parse_gnome(text: &str) -> Mode {
    match text.trim().trim_matches('\'') {
        "prefer-dark" => Mode::Dark,
        "prefer-light" => Mode::Light,
        // 'default' is light on GNOME, but other desktops leave it at default
        // while being dark, so let them answer.
        _ => Mode::Unknown,
    }
}

/// XFCE has no light/dark switch; its GTK theme's name says it (`Kali-Dark`,
/// `Adwaita-dark`).
fn xfce() -> Mode {
    output("xfconf-query", &["-c", "xsettings", "-p", "/Net/ThemeName"])
        .map_or(Mode::Unknown, |name| theme_name_mode(&name))
}

fn theme_name_mode(name: &str) -> Mode {
    if name.trim().is_empty() {
        Mode::Unknown
    } else if name.to_lowercase().contains("dark") {
        Mode::Dark
    } else {
        Mode::Light
    }
}

/// KDE Plasma: the color scheme's name (`BreezeDark`).
fn kde() -> Mode {
    ["kreadconfig6", "kreadconfig5"]
        .iter()
        .find_map(|program| {
            output(
                program,
                &[
                    "--file",
                    "kdeglobals",
                    "--group",
                    "General",
                    "--key",
                    "ColorScheme",
                ],
            )
        })
        .map_or(Mode::Unknown, |name| theme_name_mode(&name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_portal_reply() {
        assert_eq!(parse_portal("(<uint32 1>,)"), Mode::Dark);
        assert_eq!(parse_portal("(<<uint32 2>>,)"), Mode::Light);
        assert_eq!(parse_portal("(<uint32 0>,)"), Mode::Unknown);
        assert_eq!(parse_portal("garbage"), Mode::Unknown);
    }

    #[test]
    fn reads_gnome_and_theme_names() {
        assert_eq!(parse_gnome("'prefer-dark'\n"), Mode::Dark);
        assert_eq!(parse_gnome("'prefer-light'"), Mode::Light);
        assert_eq!(parse_gnome("'default'"), Mode::Unknown);
        assert_eq!(theme_name_mode("Kali-Dark"), Mode::Dark);
        assert_eq!(theme_name_mode("BreezeDark"), Mode::Dark);
        assert_eq!(theme_name_mode("Greybird"), Mode::Light);
        assert_eq!(theme_name_mode(""), Mode::Unknown);
    }
}
