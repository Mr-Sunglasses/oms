//! `oms doctor`: checks the setup and says what to fix.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::Result;

use crate::repo::Repos;
use crate::settings::{Settings, data_dir};
use crate::{apps, daemon, ghostty, update};

enum Level {
    Ok,
    Warn,
    Fail,
}

struct Report {
    problems: usize,
}

impl Report {
    fn line(&mut self, level: Level, what: &str, detail: impl AsRef<str>) {
        let mark = match level {
            Level::Ok => "\x1b[32m✓\x1b[0m",
            Level::Warn => "\x1b[33m!\x1b[0m",
            Level::Fail => {
                self.problems += 1;
                "\x1b[31m✗\x1b[0m"
            }
        };
        println!("{mark} {what:<18} {}", detail.as_ref());
    }
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    Some(
        String::from_utf8_lossy(if out.status.success() {
            &out.stdout
        } else {
            &out.stderr
        })
        .trim()
        .to_string(),
    )
}

pub fn run_checks(repos: Option<&Repos>) -> Result<()> {
    let mut r = Report { problems: 0 };
    let settings = Settings::load();

    // oms itself.
    let current = env!("CARGO_PKG_VERSION");
    match update::latest_tag() {
        Ok(tag) if update::is_newer(tag.trim_start_matches('v'), current) => r.line(
            Level::Warn,
            "oms",
            format!("{current}; {tag} is out, run `oms self-update`"),
        ),
        Ok(_) => r.line(Level::Ok, "oms", format!("{current} (latest)")),
        Err(_) => r.line(
            Level::Warn,
            "oms",
            format!("{current} (couldn't check for updates)"),
        ),
    }

    // Ghostty.
    let ghostty_bin = [
        "ghostty",
        "/Applications/Ghostty.app/Contents/MacOS/ghostty",
    ]
    .into_iter()
    .find(|b| run(b, &["--version"]).is_some());
    match ghostty_bin {
        Some(bin) => {
            let version = run(bin, &["--version"]).unwrap_or_default();
            let version = version.lines().next().unwrap_or("installed").to_string();
            r.line(Level::Ok, "Ghostty", version);
        }
        None => r.line(
            Level::Fail,
            "Ghostty",
            "not found; install it from https://ghostty.org",
        ),
    }
    if std::env::var("TERM_PROGRAM").as_deref() != Ok("ghostty") {
        r.line(
            Level::Warn,
            "Terminal",
            "not running in Ghostty, so there's no live preview here",
        );
    }

    // Config.
    let config = ghostty::config_path();
    let shown = config.display().to_string();
    if config.exists() {
        let valid = ghostty_bin.and_then(|b| {
            let out = Command::new(b)
                .arg("+validate-config")
                .arg(format!("--config-file={shown}"))
                .output()
                .ok()?;
            Some((
                out.status.success(),
                String::from_utf8_lossy(&out.stdout).trim().to_string(),
            ))
        });
        match valid {
            Some((false, msg)) => r.line(
                Level::Fail,
                "Config",
                format!("{shown}: {}", msg.lines().next().unwrap_or("has errors")),
            ),
            _ => r.line(Level::Ok, "Config", &shown),
        }
    } else {
        r.line(
            Level::Warn,
            "Config",
            format!("{shown} doesn't exist yet; oms creates it when you apply a theme"),
        );
    }
    match ghostty::current_theme(&config) {
        Some(theme) => {
            let themes_dir = config
                .parent()
                .map(|d| d.join("themes"))
                .unwrap_or_default();
            let missing: Vec<&str> = theme
                .split(',')
                .map(|t| {
                    t.trim()
                        .trim_start_matches("light:")
                        .trim_start_matches("dark:")
                })
                .filter(|t| t.starts_with("Omarchy ") && !themes_dir.join(t).exists())
                .collect();
            if missing.is_empty() {
                r.line(Level::Ok, "Theme", theme);
            } else {
                r.line(
                    Level::Fail,
                    "Theme",
                    format!("{} isn't installed; run `oms update`", missing.join(", ")),
                );
            }
        }
        None => r.line(Level::Warn, "Theme", "none set; pick one with `oms`"),
    }

    // Font from the config, if it names one.
    if let Some(font) = fs::read_to_string(&config).ok().and_then(|t| {
        t.lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .filter_map(|l| l.split_once('='))
            .find(|(k, _)| k.trim() == "font-family")
            .map(|(_, v)| v.trim().trim_matches('"').to_string())
    }) && let Some(bin) = ghostty_bin
    {
        let fonts = run(bin, &["+list-fonts"]).unwrap_or_default();
        if fonts.lines().any(|l| l.trim() == font) {
            r.line(Level::Ok, "Font", &font);
        } else {
            r.line(
                Level::Fail,
                "Font",
                format!("{font} isn't installed, so Ghostty falls back to its default"),
            );
        }
    }

    // Downloads.
    match repos {
        Some(repos) => {
            let count = |dir: &Path| {
                fs::read_dir(dir)
                    .map(|d| {
                        d.flatten()
                            .filter(|e| e.path().is_dir() || dir.ends_with("themes"))
                            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                            .count()
                    })
                    .unwrap_or(0)
            };
            r.line(
                Level::Ok,
                "Downloads",
                format!(
                    "{} themes, {} wallpaper folders in {}",
                    count(&repos.themes.join("themes")),
                    count(&repos.wallpapers),
                    data_dir().display()
                ),
            );
        }
        None => r.line(Level::Fail, "Downloads", "missing; run `oms update`"),
    }

    // Wallpapers on every Space.
    let store = dirs::data_dir()
        .unwrap_or_default()
        .join("com.apple.wallpaper/Store/Index.plist");
    if store.exists() {
        r.line(Level::Ok, "Wallpapers", "set on every Space and display");
    } else {
        r.line(
            Level::Warn,
            "Wallpapers",
            "only the current Space (needs macOS 14 or newer for all Spaces)",
        );
    }

    // Background agent.
    let wanted = settings.needs_daemon();
    let loaded = run(
        "launchctl",
        &[
            "print",
            &format!(
                "gui/{}/{}",
                run("id", &["-u"]).unwrap_or_default(),
                daemon::label()
            ),
        ],
    )
    .is_some_and(|out| out.contains("state = running"));
    match (wanted, daemon::is_installed(), loaded) {
        (false, false, _) => r.line(
            Level::Ok,
            "Agent",
            "not needed (light/dark switching and rotation are off)",
        ),
        (true, true, true) => r.line(Level::Ok, "Agent", "running"),
        (true, _, _) => r.line(
            Level::Fail,
            "Agent",
            "should be running but isn't; run `oms rotate off` then turn it back on",
        ),
        (false, true, _) => r.line(Level::Warn, "Agent", "installed but not needed"),
    }

    // Themed apps.
    for app in &settings.apps {
        if apps::available(app) {
            r.line(Level::Ok, &format!("App: {app}"), "follows the theme");
        } else {
            r.line(
                Level::Warn,
                &format!("App: {app}"),
                "turned on but doesn't look installed",
            );
        }
    }

    println!();
    if r.problems == 0 {
        println!("Everything looks good.");
    } else {
        println!(
            "{} problem{} found.",
            r.problems,
            if r.problems == 1 { "" } else { "s" }
        );
    }
    Ok(())
}
