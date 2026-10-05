//! `oms doctor`: checks the setup and says what to fix.

use std::fs;
use std::path::Path;
use std::process::Command;

use crate::repo::Repos;
use crate::settings::{Settings, data_dir};
use crate::{apps, daemon, ghostty, update};

#[derive(Clone, Copy)]
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

pub fn run_checks(repos: Option<&Repos>) {
    let mut r = Report { problems: 0 };
    let settings = Settings::load();
    let config = ghostty::config_path();

    check_oms(&mut r);
    let ghostty_bin = check_ghostty(&mut r);
    check_config(&mut r, ghostty_bin, &config);
    check_font(&mut r, ghostty_bin, &config);
    check_downloads(&mut r, repos);
    check_wallpapers(&mut r);
    check_agent(&mut r, &settings);
    check_other_agents(&mut r);
    check_apps(&mut r, &settings);

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
}

/// The installed version, and whether a newer one is out.
fn check_oms(r: &mut Report) {
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
}

/// Ghostty installed (and which binary), and whether we're running in it.
fn check_ghostty(r: &mut Report) -> Option<&'static str> {
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
    ghostty_bin
}

/// The config file is valid and its theme is installed.
fn check_config(r: &mut Report, ghostty_bin: Option<&str>, config: &Path) {
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
    match ghostty::current_theme(config) {
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
}

/// The font the config names is installed.
fn check_font(r: &mut Report, ghostty_bin: Option<&str>, config: &Path) {
    if let Some(font) = fs::read_to_string(config).ok().and_then(|t| {
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
}

/// The themes and wallpapers have been downloaded.
fn check_downloads(r: &mut Report, repos: Option<&Repos>) {
    match repos {
        Some(repos) => {
            let count = |dir: &Path| {
                fs::read_dir(dir).map_or(0, |d| {
                    d.flatten()
                        .filter(|e| e.path().is_dir() || dir.ends_with("themes"))
                        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                        .count()
                })
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
}

/// Wallpapers can be set on every Space (macOS 14+).
fn check_wallpapers(r: &mut Report) {
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
}

/// The background agent is running exactly when it's needed.
fn check_agent(r: &mut Report, settings: &Settings) {
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
}

/// No other oms agent is left behind (e.g. by a run with a different
/// `OMS_DATA_DIR`); one would keep switching the wallpaper on its own.
fn check_other_agents(r: &mut Report) {
    let uid = run("id", &["-u"]).unwrap_or_default();
    for agent in daemon::other_agents() {
        let program = match &agent.program {
            Some(p) if p.exists() => p.display().to_string(),
            Some(p) => format!("{} (missing)", p.display()),
            None => "unknown binary".to_string(),
        };
        r.line(
            Level::Fail,
            "Other agent",
            format!(
                "{} runs {program} and can change your wallpaper; remove it with\n{:21}launchctl bootout gui/{uid}/{}; rm '{}'",
                agent.label,
                "",
                agent.label,
                agent.plist.display()
            ),
        );
    }
}

/// Apps that are switched on look installed.
fn check_apps(r: &mut Report, settings: &Settings) {
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
}
