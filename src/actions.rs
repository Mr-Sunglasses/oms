//! Applying themes and wallpapers, shared by the TUI, the CLI and the agent.

use std::fmt::Write as _;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::repo::Theme;
use crate::settings::{Choice, Settings};
use crate::{apps, daemon, ghostty, wallpaper};

/// What happened, for a status line.
pub struct Outcome {
    pub message: String,
}

/// Applies a theme to Ghostty and the enabled apps, and remembers it. This is
/// a single fixed theme, so it turns off light/dark switching.
pub fn apply_theme(theme: &Theme, settings: &mut Settings) -> Result<Outcome> {
    let apps = theme_effects(theme, &settings.apps)?;
    let note = record_theme(theme, settings)?;
    Ok(Outcome {
        message: format!("Applied {}{apps}{note}", theme.name),
    })
}

/// The slow part of applying a theme (files and apps); safe off the main thread.
/// Returns text for a status line.
pub fn theme_effects(theme: &Theme, apps: &[String]) -> Result<String> {
    ghostty::set_theme(&ghostty::config_path(), &theme.ghostty_name)?;
    Ok(apps_text(apps::apply(theme, apps)))
}

/// Remembers an applied theme. Returns a note for the status line.
pub fn record_theme(theme: &Theme, settings: &mut Settings) -> Result<String> {
    let mut note = String::new();
    if settings.auto {
        settings.auto = false;
        note.push_str(". Light/dark switching is off");
    }
    settings.record_recent(&theme.slug);
    settings.save()?;
    daemon::sync(settings)?;
    Ok(note)
}

fn apps_text((done, failed): (Vec<String>, Vec<String>)) -> String {
    let mut text = String::new();
    if !done.is_empty() {
        let _ = write!(text, " (+ {})", done.join(", "));
    }
    if !failed.is_empty() {
        let _ = write!(text, ". Couldn't theme {}", failed.join("; "));
    }
    text
}

/// Applies the enabled app themes. Returns text to add to a status line.
pub fn apply_apps(theme: &Theme, settings: &Settings) -> String {
    apps_text(apps::apply(theme, &settings.apps))
}

pub fn apply_wallpaper(theme: &Theme, index: usize, settings: &mut Settings) -> Result<PathBuf> {
    let path = theme
        .wallpapers
        .get(index)
        .with_context(|| format!("{} has no wallpaper {}", theme.name, index + 1))?
        .clone();
    wallpaper::set(&path)?;
    settings.wallpaper_index.insert(theme.slug.clone(), index);
    settings.save()?;
    Ok(path)
}

/// Sets the theme for light or dark mode. Once both are set, Ghostty follows
/// the system light/dark mode and the agent switches wallpapers and app themes.
pub fn set_mode_theme(
    themes: &[Theme],
    dark: bool,
    choice: Choice,
    settings: &mut Settings,
) -> Result<Outcome> {
    let name = find(themes, &choice.theme)?.name.clone();
    if dark {
        settings.dark = Some(choice);
    } else {
        settings.light = Some(choice);
    }
    let mode = if dark { "dark" } else { "light" };
    let message = if let (Some(light), Some(dark_choice)) = (&settings.light, &settings.dark) {
        settings.auto = true;
        let light_theme = find(themes, &light.theme)?;
        let dark_theme = find(themes, &dark_choice.theme)?;
        ghostty::set_theme(
            &ghostty::config_path(),
            &ghostty::light_dark_value(
                &light_theme.ghostty_name,
                &dark_theme.ghostty_name,
                daemon::is_dark(),
            ),
        )?;
        format!(
            "{name} is the {mode} theme. Following the system: {} by day, {} by night",
            light_theme.name, dark_theme.name
        )
    } else {
        let other = if dark { "light (L)" } else { "dark (D)" };
        format!("{name} is the {mode} theme. Now pick a {other} theme")
    };
    settings.save()?;
    daemon::sync(settings)?;
    Ok(Outcome { message })
}

pub fn find<'a>(themes: &'a [Theme], query: &str) -> Result<&'a Theme> {
    themes
        .iter()
        .find(|t| t.matches(query))
        .with_context(|| format!("no theme called {query:?} (see `oms list`)"))
}

/// Parses "3", "random" or nothing (the remembered one, else the first).
pub fn wallpaper_index(theme: &Theme, which: Option<&str>, settings: &Settings) -> Result<usize> {
    let count = theme.wallpapers.len();
    if count == 0 {
        anyhow::bail!("{} has no wallpapers", theme.name);
    }
    Ok(match which {
        None => settings
            .wallpaper_index
            .get(&theme.slug)
            .copied()
            .filter(|&i| i < count)
            .unwrap_or(0),
        Some("random") => random_below(count),
        Some(n) => match n.parse::<usize>() {
            Ok(n) if (1..=count).contains(&n) => n - 1,
            _ => anyhow::bail!("{} has wallpapers 1 to {count}", theme.name),
        },
    })
}

/// A random number in `0..n` (0 when `n` is 0).
pub fn random_below(n: usize) -> usize {
    use rand::Rng;
    rand::thread_rng().gen_range(0..n.max(1))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A theme with `wallpapers` pictures and default colors, for tests.
    pub(crate) fn theme(slug: &str, name: &str, wallpapers: usize) -> Theme {
        Theme {
            name: name.into(),
            ghostty_name: format!("Omarchy {name}"),
            slug: slug.into(),
            background: (0, 0, 0),
            foreground: (255, 255, 255),
            cursor: (255, 255, 255),
            palette: [(0, 0, 0); 16],
            accent: (0, 0, 0),
            wallpapers: (1..=wallpapers)
                .map(|i| PathBuf::from(format!("/w/{slug}/{i}.jpg")))
                .collect(),
            builtin_wallpapers: wallpapers,
            apps_dir: PathBuf::new(),
        }
    }

    #[test]
    fn picks_wallpapers_by_number_random_or_memory() {
        let nord = theme("nord", "Nord", 3);
        let mut settings = Settings::default();
        assert_eq!(wallpaper_index(&nord, None, &settings).unwrap(), 0);
        assert_eq!(wallpaper_index(&nord, Some("3"), &settings).unwrap(), 2);
        assert!(wallpaper_index(&nord, Some("0"), &settings).is_err());
        assert!(wallpaper_index(&nord, Some("4"), &settings).is_err());
        assert!(wallpaper_index(&nord, Some("two"), &settings).is_err());
        for _ in 0..50 {
            assert!(wallpaper_index(&nord, Some("random"), &settings).unwrap() < 3);
        }

        settings.wallpaper_index.insert("nord".into(), 1);
        assert_eq!(wallpaper_index(&nord, None, &settings).unwrap(), 1);
        // A remembered index past the end (a wallpaper was removed) falls back.
        settings.wallpaper_index.insert("nord".into(), 7);
        assert_eq!(wallpaper_index(&nord, None, &settings).unwrap(), 0);

        assert!(wallpaper_index(&theme("bare", "Bare", 0), None, &settings).is_err());
    }

    #[test]
    fn random_stays_in_range() {
        assert_eq!(random_below(0), 0);
        assert_eq!(random_below(1), 0);
        let seen: std::collections::HashSet<usize> = (0..200).map(|_| random_below(4)).collect();
        assert_eq!(seen.len(), 4, "{seen:?}");
    }

    #[test]
    fn finds_themes_by_slug_or_name() {
        let themes = [
            theme("nord", "Nord", 1),
            theme("tokyo-night", "Tokyo Night", 1),
        ];
        assert_eq!(find(&themes, "tokyo-night").unwrap().slug, "tokyo-night");
        assert_eq!(find(&themes, "Tokyo Night").unwrap().slug, "tokyo-night");
        assert_eq!(find(&themes, "omarchy nord").unwrap().slug, "nord");
        assert!(find(&themes, "tokyo").is_err());
    }
}
