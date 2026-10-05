//! The command-line subcommands: status, auto, rotate, apps and wallpapers.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::repo::{self, Repos, Theme};
use crate::settings::{Choice, Settings};
use crate::{actions, apps, daemon, ghostty, wallpaper};

pub fn status(repos: &Repos, settings: &Settings) -> Result<()> {
    let themes = repos.load(settings)?;
    let name = |slug: &str| {
        themes
            .iter()
            .find(|t| t.slug == slug)
            .map(|t| t.name.clone())
            .unwrap_or(slug.to_string())
    };
    let theme = ghostty::current_theme(&ghostty::config_path()).unwrap_or_else(|| "(none)".into());
    println!("Ghostty theme:    {theme}");
    if let Some(path) = wallpaper::current() {
        println!("Wallpaper:        {}", path.display());
    }
    match (&settings.light, &settings.dark, settings.auto) {
        (Some(l), Some(d), true) => {
            println!(
                "Light/dark:       on ({} / {})",
                name(&l.theme),
                name(&d.theme)
            );
        }
        _ => println!("Light/dark:       off"),
    }
    match settings.rotate_minutes {
        Some(m) => println!("Rotation:         every {}", format_minutes(m)),
        None => println!("Rotation:         off"),
    }
    let apps = if settings.apps.is_empty() {
        "none".to_string()
    } else {
        settings.apps.join(", ")
    };
    println!("Themed apps:      {apps}");
    println!(
        "Background agent: {}",
        if daemon::is_installed() {
            "running"
        } else {
            "not needed"
        }
    );
    if !settings.favorites.is_empty() {
        let favs: Vec<String> = settings.favorites.iter().map(|f| name(f)).collect();
        println!("Favorites:        {}", favs.join(", "));
    }
    Ok(())
}

/// "nord:2" -> the Nord theme with its 2nd wallpaper.
fn parse_choice(themes: &[Theme], spec: &str, settings: &Settings) -> Result<Choice> {
    let (name, which) = match spec.rsplit_once(':') {
        Some((n, w)) if w.parse::<usize>().is_ok() => (n, Some(w)),
        _ => (spec, None),
    };
    let theme = actions::find(themes, name)?;
    let wallpaper = if theme.wallpapers.is_empty() {
        0
    } else {
        actions::wallpaper_index(theme, which, settings)?
    };
    Ok(Choice {
        theme: theme.slug.clone(),
        wallpaper,
    })
}

pub fn auto(repos: &Repos, settings: &mut Settings, args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        None => {
            println!(
                "Light/dark switching is {}. Use `oms auto <light> <dark>` or `oms auto off`.",
                if settings.auto { "on" } else { "off" }
            );
            Ok(())
        }
        Some("off") => {
            settings.auto = false;
            settings.save()?;
            daemon::sync(settings)?;
            // Pin Ghostty to the theme showing now.
            let themes = repos.load(settings)?;
            let choice = if daemon::is_dark() {
                &settings.dark
            } else {
                &settings.light
            };
            if let Some(choice) = choice
                && let Ok(theme) = actions::find(&themes, &choice.theme)
            {
                ghostty::set_theme(&ghostty::config_path(), &theme.ghostty_name)?;
            }
            println!("Light/dark switching is off.");
            Ok(())
        }
        Some(light) => {
            let dark = args
                .get(1)
                .context("usage: oms auto <light-theme> <dark-theme>")?;
            let themes = repos.load(settings)?;
            let light = parse_choice(&themes, light, settings)?;
            let dark = parse_choice(&themes, dark, settings)?;
            actions::set_mode_theme(&themes, false, light, settings)?;
            let outcome = actions::set_mode_theme(&themes, true, dark, settings)?;
            println!("{}.", outcome.message);
            Ok(())
        }
    }
}

fn parse_minutes(text: &str) -> Result<u64> {
    let text = text.trim().to_lowercase();
    let (number, unit) = text.split_at(
        text.find(|c: char| !c.is_ascii_digit())
            .unwrap_or(text.len()),
    );
    let n: u64 = number
        .parse()
        .with_context(|| format!("can't read {text:?}; try 30m or 2h"))?;
    let minutes = match unit {
        "" | "m" | "min" | "mins" => n,
        "h" | "hr" | "hour" | "hours" => n * 60,
        "d" | "day" | "days" => n * 60 * 24,
        _ => bail!("can't read {text:?}; try 30m or 2h"),
    };
    if minutes == 0 {
        bail!("the interval must be at least a minute");
    }
    Ok(minutes)
}

fn format_minutes(m: u64) -> String {
    if m.is_multiple_of(60) {
        format!("{}h", m / 60)
    } else {
        format!("{m}m")
    }
}

pub fn rotate(settings: &mut Settings, interval: Option<&str>) -> Result<()> {
    match interval {
        None => {
            match settings.rotate_minutes {
                Some(m) => println!(
                    "Wallpapers rotate every {}. Stop with `oms rotate off`.",
                    format_minutes(m)
                ),
                None => println!("Rotation is off. Start it with e.g. `oms rotate 30m`."),
            }
            return Ok(());
        }
        Some("off") => {
            settings.rotate_minutes = None;
            println!("Rotation is off.");
        }
        Some(text) => {
            let m = parse_minutes(text)?;
            settings.rotate_minutes = Some(m);
            println!(
                "The wallpaper now changes to the theme's next one every {}.",
                format_minutes(m)
            );
        }
    }
    settings.save()?;
    daemon::sync(settings)
}

pub fn apps_command(repos: &Repos, settings: &mut Settings, args: &[String]) -> Result<()> {
    let names = args.get(1..).unwrap_or_default();
    match args.first().map(String::as_str) {
        None => {
            for (id, about) in apps::APPS {
                let state = if settings.app_enabled(id) {
                    "on "
                } else if apps::available(id) {
                    "off"
                } else {
                    "-- "
                };
                println!("{state} {id:<7} {about}");
            }
            println!(
                "\n(-- means it doesn't look installed.) Turn one on with `oms apps on <app>`."
            );
            Ok(())
        }
        Some("on") => {
            if names.is_empty() {
                bail!("which apps? e.g. `oms apps on nvim btop`");
            }
            for name in names {
                if !apps::is_known(name) {
                    bail!("unknown app {name:?}; see `oms apps`");
                }
                if !settings.app_enabled(name) {
                    settings.apps.push(name.clone());
                }
            }
            settings.save()?;
            // Theme them right away with the current theme.
            let themes = repos.load(settings)?;
            match active_theme(&themes, settings) {
                Some(theme) => {
                    let (done, failed) = apps::apply(theme, names);
                    if !done.is_empty() {
                        println!("Themed {} with {}.", done.join(", "), theme.name);
                    }
                    for f in failed {
                        println!("Couldn't theme {f}");
                    }
                }
                None => println!("Turned on. They'll follow the next theme you apply."),
            }
            Ok(())
        }
        Some("off") => {
            for name in names {
                apps::remove(name)?;
                settings.apps.retain(|a| a != name);
            }
            settings.save()?;
            println!(
                "Turned off {} and removed their oms themes.",
                names.join(", ")
            );
            Ok(())
        }
        Some(other) => bail!("unknown `oms apps {other}`; use on or off"),
    }
}

/// The theme showing now: the light/dark one if switching is on, else Ghostty's.
fn active_theme<'a>(themes: &'a [Theme], settings: &Settings) -> Option<&'a Theme> {
    if settings.auto {
        let choice = if daemon::is_dark() {
            &settings.dark
        } else {
            &settings.light
        };
        if let Some(choice) = choice {
            return actions::find(themes, &choice.theme).ok();
        }
    }
    let current = ghostty::current_theme(&ghostty::config_path())?;
    themes.iter().find(|t| t.ghostty_name == current)
}

pub fn wallpapers(repos: &Repos, settings: &mut Settings, args: &[String]) -> Result<()> {
    let themes = repos.load(settings)?;
    let theme = actions::find(&themes, args.get(1).context("which theme? See `oms list`")?)?;
    let paths: Vec<PathBuf> = args
        .get(2..)
        .unwrap_or_default()
        .iter()
        .map(|p| {
            PathBuf::from(p)
                .canonicalize()
                .with_context(|| format!("{p} not found"))
        })
        .collect::<Result<_>>()?;
    match args.first().map(String::as_str) {
        Some("list") => {
            for (i, path) in theme.wallpapers.iter().enumerate() {
                let mark = if theme.is_custom_wallpaper(i) {
                    " (yours)"
                } else {
                    ""
                };
                println!("{:>2} {}{mark}", i + 1, path.display());
            }
            Ok(())
        }
        Some("add") => {
            if paths.is_empty() {
                bail!("add which pictures or folders?");
            }
            let found = repo::expand_wallpapers(&paths).len();
            if found == 0 {
                bail!("no pictures (jpg, png, heic, …) found there");
            }
            let list = settings
                .custom_wallpapers
                .entry(theme.slug.clone())
                .or_default();
            for p in paths {
                if !list.contains(&p) {
                    list.push(p);
                }
            }
            settings.save()?;
            println!("Added {found} pictures to {}.", theme.name);
            Ok(())
        }
        Some("remove") => {
            if let Some(list) = settings.custom_wallpapers.get_mut(&theme.slug) {
                list.retain(|p| !paths.contains(p));
                if list.is_empty() {
                    settings.custom_wallpapers.remove(&theme.slug);
                }
            }
            settings.save()?;
            println!("Removed them from {}.", theme.name);
            Ok(())
        }
        _ => bail!("use `oms wallpapers add|remove|list <theme> ...`"),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_minutes;

    #[test]
    fn reads_intervals() {
        assert_eq!(parse_minutes("30m").unwrap(), 30);
        assert_eq!(parse_minutes("45").unwrap(), 45);
        assert_eq!(parse_minutes("2h").unwrap(), 120);
        assert_eq!(parse_minutes("1d").unwrap(), 1440);
        assert!(parse_minutes("0").is_err());
        assert!(parse_minutes("soon").is_err());
    }
}
