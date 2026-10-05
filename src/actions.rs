//! Applying themes and wallpapers, shared by the TUI, the CLI and the agent.

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
    ghostty::set_theme(&ghostty::config_path(), &theme.ghostty_name)?;
    let mut message = format!("Applied {}", theme.name);
    message.push_str(&apply_apps(theme, settings));
    if settings.auto {
        settings.auto = false;
        message.push_str(". Light/dark switching is off");
    }
    settings.record_recent(&theme.slug);
    settings.save()?;
    daemon::sync(settings)?;
    Ok(Outcome { message })
}

/// Applies the enabled app themes. Returns text to add to a status line.
pub fn apply_apps(theme: &Theme, settings: &Settings) -> String {
    let (done, failed) = apps::apply(theme, &settings.apps);
    let mut text = String::new();
    if !done.is_empty() {
        text.push_str(&format!(" (+ {})", done.join(", ")));
    }
    if !failed.is_empty() {
        text.push_str(&format!(". Couldn't theme {}", failed.join("; ")));
    }
    text
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
/// the macOS appearance and the agent switches wallpapers and app themes.
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
    let message = match (&settings.light, &settings.dark) {
        (Some(light), Some(dark_choice)) => {
            settings.auto = true;
            let light_theme = find(themes, &light.theme)?;
            let dark_theme = find(themes, &dark_choice.theme)?;
            ghostty::set_theme(
                &ghostty::config_path(),
                &format!(
                    "light:{},dark:{}",
                    light_theme.ghostty_name, dark_theme.ghostty_name
                ),
            )?;
            format!(
                "{name} is the {mode} theme. Following macOS: {} by day, {} by night",
                light_theme.name, dark_theme.name
            )
        }
        _ => {
            let other = if dark { "light (L)" } else { "dark (D)" };
            format!("{name} is the {mode} theme. Now pick a {other} theme")
        }
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

pub fn random_below(n: usize) -> usize {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize ^ d.as_secs() as usize)
        .unwrap_or(0);
    nanos % n.max(1)
}
