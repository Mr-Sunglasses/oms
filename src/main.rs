//! oms: pick an Omarchy theme for Ghostty and a matching macOS
//! wallpaper from one TUI.

mod actions;
mod app;
mod apps;
mod daemon;
mod ghostty;
mod images;
mod preset;
mod repo;
mod settings;
mod ui;
mod uninstall;
mod update;
mod wallpaper;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui_image::picker::Picker;

use crate::app::App;
use crate::repo::{Repos, Theme};
use crate::settings::{Choice, Settings};

const USAGE: &str = "\
oms: Omarchy themes for Ghostty and matching macOS wallpapers

Usage:
  oms                               open the picker (press ? inside for keys)
  oms list                          list themes (★ favorites)
  oms apply <theme> [n|random]      apply a theme and one of its wallpapers
  oms status                        show what's applied and what's switched on

Light and dark:
  oms auto <light> <dark>           follow macOS light/dark mode with two themes
                                    (pick wallpapers with theme:n, e.g. nord:2)
  oms auto off                      stop following light/dark mode

Wallpapers:
  oms rotate <30m|2h|off>           change to the theme's next wallpaper on a timer
  oms wallpapers add <theme> <path>...      add your own pictures or folders
  oms wallpapers remove <theme> <path>...   remove them again
  oms wallpapers list <theme>               list a theme's wallpapers

Other apps:
  oms apps                          list apps that can follow the theme
  oms apps on <app>...              theme them too: nvim btop bat tmux accent
  oms apps off <app>...             stop theming them (and undo it)

Ghostty config:
  oms config install                use Kanishk's Ghostty config (backs up yours)
  oms config install --only <a,b>   just some sections (see `oms config sections`)
  oms config show [section]         print the config
  oms config restore                put your previous Ghostty config back

Maintenance:
  oms update                        download the latest themes and wallpapers
  oms self-update                   update oms itself to the latest release
  oms uninstall [--all]             remove oms (--all: Omarchy theme files too)

Options:
  --themes <dir>       use a ghostty-omarchy-themes checkout
  --wallpapers <dir>   use an omarchy-wallpapers checkout
  -h, --help           show this help
  -V, --version        show the version

Themes can be given as a name or slug: \"Tokyo Night\", tokyo-night.";

fn main() -> Result<()> {
    let mut args = Vec::new();
    let (mut themes_dir, mut wallpapers_dir) = (None, None);
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("oms {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--themes" => {
                themes_dir = Some(PathBuf::from(
                    iter.next().context("--themes needs a directory")?,
                ))
            }
            "--wallpapers" => {
                wallpapers_dir = Some(PathBuf::from(
                    iter.next().context("--wallpapers needs a directory")?,
                ))
            }
            _ => args.push(arg),
        }
    }
    let arg = |i: usize| args.get(i).map(String::as_str);

    // Commands that don't need the theme repos.
    match arg(0) {
        Some("self-update" | "selfupdate") => return update::self_update(),
        Some("self") if arg(1) == Some("update") => return update::self_update(),
        Some("uninstall") => return uninstall::run(&args[1..]),
        Some("daemon") => return daemon::run(),
        Some("config") if arg(1) != Some("install") => return preset::run(&args[1..]),
        _ => {}
    }

    let repos = Repos::locate(themes_dir, wallpapers_dir)?;
    ghostty::install_themes(&repos.themes)?;
    let mut settings = Settings::load();

    match arg(0) {
        None => tui(&repos, settings),
        Some("config") => preset::run(&args[1..]),
        Some("update") => {
            let changed = repos.update()?;
            ghostty::install_themes(&repos.themes)?;
            println!(
                "{}",
                if changed {
                    "Downloaded the latest themes and wallpapers."
                } else {
                    "Already up to date."
                }
            );
            Ok(())
        }
        Some("list") => {
            for t in repos.load(&settings)? {
                let star = if settings.is_favorite(&t.slug) {
                    "★"
                } else {
                    " "
                };
                println!(
                    "{star} {:<18} {:<20} {} wallpapers",
                    t.slug,
                    t.name,
                    t.wallpapers.len()
                );
            }
            Ok(())
        }
        Some("apply") => {
            let themes = repos.load(&settings)?;
            let theme = actions::find(&themes, arg(1).context("which theme? See `oms list`")?)?;
            let outcome = actions::apply_theme(theme, &mut settings)?;
            println!("{}.", outcome.message);
            if !theme.wallpapers.is_empty() {
                let index = actions::wallpaper_index(theme, arg(2), &settings)?;
                let path = actions::apply_wallpaper(theme, index, &mut settings)?;
                println!("Wallpaper: {}", path.display());
            }
            Ok(())
        }
        Some("status") => status(&repos, &settings),
        Some("auto") => auto(&repos, &mut settings, &args[1..]),
        Some("rotate") => rotate(&mut settings, arg(1)),
        Some("apps") => apps_command(&repos, &mut settings, &args[1..]),
        Some("wallpapers") => wallpapers(&repos, &mut settings, &args[1..]),
        Some(other) => bail!("unknown command {other:?}\n\n{USAGE}"),
    }
}

fn tui(repos: &Repos, settings: Settings) -> Result<()> {
    let themes = repos.load(&settings)?;
    let mut terminal = ratatui::init();
    let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
    let mut app = App::new(themes, settings, picker, repos);
    let result = run(&mut terminal, &mut app);
    let restored = app.restore();
    ratatui::restore();
    result.and(restored)
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.quit {
        app.tick();
        terminal.draw(|f| ui::draw(f, app))?;
        if event::poll(Duration::from_millis(40))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.handle_key(key);
        }
    }
    Ok(())
}

fn status(repos: &Repos, settings: &Settings) -> Result<()> {
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
            )
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

fn auto(repos: &Repos, settings: &mut Settings, args: &[String]) -> Result<()> {
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

fn rotate(settings: &mut Settings, interval: Option<&str>) -> Result<()> {
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

fn apps_command(repos: &Repos, settings: &mut Settings, args: &[String]) -> Result<()> {
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

fn wallpapers(repos: &Repos, settings: &mut Settings, args: &[String]) -> Result<()> {
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
