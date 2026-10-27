//! What oms remembers between runs: favorites, history, light/dark themes,
//! wallpaper rotation, app theming and the user's own wallpapers.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const RECENT_MAX: usize = 6;

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Settings {
    /// Favorite theme slugs.
    #[serde(default)]
    pub favorites: Vec<String>,
    /// Recently applied theme slugs, newest first.
    #[serde(default)]
    pub recent: Vec<String>,
    /// Themes (and wallpapers) to use in light and dark mode.
    #[serde(default)]
    pub light: Option<Choice>,
    #[serde(default)]
    pub dark: Option<Choice>,
    /// Whether to follow the macOS appearance with `light` and `dark`.
    #[serde(default)]
    pub auto: bool,
    /// Minutes between wallpaper changes, if rotation is on.
    #[serde(default)]
    pub rotate_minutes: Option<u64>,
    /// Apps that follow the theme: nvim, btop, bat, tmux, accent.
    #[serde(default)]
    pub apps: Vec<String>,
    /// Extra wallpaper files or folders per theme slug.
    #[serde(default)]
    pub custom_wallpapers: BTreeMap<String, Vec<PathBuf>>,
    /// Last chosen wallpaper (index) per theme slug.
    #[serde(default)]
    pub wallpaper_index: BTreeMap<String, usize>,
    /// Unix time of the last background check for updates.
    #[serde(default)]
    pub last_update_check: u64,
    /// Newest oms release seen by that check.
    #[serde(default)]
    pub latest_version: Option<String>,
    /// A release the user chose not to update to; they aren't asked again.
    #[serde(default)]
    pub skipped_version: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct Choice {
    pub theme: String,
    #[serde(default)]
    pub wallpaper: usize,
}

/// Where oms keeps its downloads and settings (`OMS_DATA_DIR` overrides it).
pub fn data_dir() -> PathBuf {
    std::env::var_os("OMS_DATA_DIR").map_or_else(
        || dirs::data_dir().unwrap_or_default().join("omarchy-switch"),
        PathBuf::from,
    )
}

fn path() -> PathBuf {
    data_dir().join("settings.json")
}

impl Settings {
    pub fn load() -> Self {
        let mut settings: Self = fs::read_to_string(path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        if settings.follow_renames() {
            let _ = settings.save();
        }
        settings
    }

    /// Points saved theme slugs at their new names after Omarchy renamed or
    /// replaced a theme. Returns whether anything changed.
    fn follow_renames(&mut self) -> bool {
        let mut changed = false;
        let mut follow = |slug: &mut String| {
            if let Some(new) = crate::repo::renamed(slug) {
                *slug = new.to_string();
                changed = true;
            }
        };
        self.favorites.iter_mut().for_each(&mut follow);
        self.recent.iter_mut().for_each(&mut follow);
        for choice in [&mut self.light, &mut self.dark].into_iter().flatten() {
            follow(&mut choice.theme);
        }
        for r in crate::repo::RENAMED {
            if let Some(index) = self.wallpaper_index.remove(r.old_slug) {
                self.wallpaper_index
                    .entry(r.new_slug.into())
                    .or_insert(index);
                changed = true;
            }
            if let Some(paths) = self.custom_wallpapers.remove(r.old_slug) {
                self.custom_wallpapers
                    .entry(r.new_slug.into())
                    .or_default()
                    .extend(paths);
                changed = true;
            }
        }
        // A renamed theme may already have been there under its new name.
        for list in [&mut self.favorites, &mut self.recent] {
            let mut seen = HashSet::new();
            list.retain(|slug| seen.insert(slug.clone()));
        }
        changed
    }

    pub fn save(&self) -> Result<()> {
        let path = path();
        fs::create_dir_all(path.parent().context("bad settings path")?)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(self)? + "\n")?;
        fs::rename(&tmp, &path)?;
        // Let the background agent pick up the change right away.
        crate::daemon::post_settings_changed();
        Ok(())
    }

    /// A known release newer than this one (shown as a reminder in the picker).
    pub fn newer_release(&self) -> Option<String> {
        let latest = self.latest_version.as_ref()?;
        crate::update::is_newer(latest, env!("CARGO_PKG_VERSION")).then(|| latest.clone())
    }

    /// A newer release to offer, unless the user skipped it.
    pub fn update_to_offer(&self) -> Option<String> {
        let latest = self.latest_version.as_ref()?;
        (crate::update::is_newer(latest, env!("CARGO_PKG_VERSION"))
            && self.skipped_version.as_ref() != Some(latest))
        .then(|| latest.clone())
    }

    pub fn is_favorite(&self, slug: &str) -> bool {
        self.favorites.iter().any(|f| f == slug)
    }

    pub fn toggle_favorite(&mut self, slug: &str) -> bool {
        if self.is_favorite(slug) {
            self.favorites.retain(|f| f != slug);
            false
        } else {
            self.favorites.push(slug.to_string());
            true
        }
    }

    pub fn record_recent(&mut self, slug: &str) {
        self.recent.retain(|r| r != slug);
        self.recent.insert(0, slug.to_string());
        self.recent.truncate(RECENT_MAX);
    }

    pub fn app_enabled(&self, app: &str) -> bool {
        self.apps.iter().any(|a| a == app)
    }

    /// Whether the background agent has anything to do.
    pub fn needs_daemon(&self) -> bool {
        (self.auto && self.light.is_some() && self.dark.is_some()) || self.rotate_minutes.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_renamed_themes() {
        let mut settings = Settings {
            favorites: vec!["osaka-jade".into(), "jade-horse".into(), "nord".into()],
            recent: vec!["nord".into(), "osaka-jade".into()],
            dark: Some(Choice {
                theme: "osaka-jade".into(),
                wallpaper: 2,
            }),
            wallpaper_index: BTreeMap::from([("osaka-jade".into(), 2)]),
            ..Settings::default()
        };
        assert!(settings.follow_renames());
        assert_eq!(settings.favorites, ["jade-horse", "nord"]);
        assert_eq!(settings.recent, ["nord", "jade-horse"]);
        assert_eq!(
            settings.dark.as_ref().map(|c| c.theme.as_str()),
            Some("jade-horse")
        );
        assert_eq!(settings.wallpaper_index.get("jade-horse"), Some(&2));
        assert!(!settings.follow_renames());
    }
}
